# Verifying a Quayflow release

Every Quayflow release publishes these assets:

| Asset | What it is |
|---|---|
| `quayflow.exe` | Engine and command-line tool (Windows x64) |
| `quayflow-bg.exe` | Windowless launcher used by the sign-in startup entry (Windows) |
| `quayflow-linux-x86_64` | Engine and command-line tool (Linux x86_64; from v0.1.5) |
| `quayflow-macos-arm64`, `quayflow-macos-x86_64` | Engine and command-line tool (macOS on Apple silicon and Intel; from v0.1.6) |
| `install.ps1` | Installer (Windows) |
| `install.sh` | Installer (Linux from v0.1.5; macOS from v0.1.6) |
| `Quayflow.psm1` | PowerShell integration (also embedded in `quayflow.exe`) |
| `SOURCE_COMMIT` | The commit the release was built from (releases after v0.1.0) |
| `SHA256SUMS` | SHA-256 hash of each asset above, in `sha256sum` format |

Each release is built by this repository's release workflow (`.github/workflows/release.yml`) and has these integrity properties:

1. **Published hash.** `SHA256SUMS` lists the SHA-256 of every other asset. The installer and `quayflow update` refuse a file whose hash does not match.
2. **Build provenance.** A GitHub artifact attestation, signed through GitHub's Sigstore integration, records that each asset was built by this repository's workflow from a specific commit and tag. Releases from v0.1.5 on only (see below).
3. **Reproducible build.** For the executables (`quayflow.exe`, `quayflow-bg.exe`, `quayflow-linux-x86_64`, and the two macOS programs), the workflow builds the same commit twice, in separate jobs and directories, and fails if the two builds differ. Windows builds on GitHub's Windows runner, Linux on Ubuntu 22.04, macOS on `macos-15`. The toolchain is pinned in `rust-toolchain.toml`, dependencies are locked with `Cargo.lock`, paths are remapped, and `SOURCE_DATE_EPOCH` is set from the tag's commit. On Windows, `/Brepro` also makes the PE header deterministic. On macOS, `-Wl,-S` leaves out the debug map, whose absolute paths would otherwise change the binary's `LC_UUID`.

The installer checks (1) always, and (2) unless you pass `-SkipAttestation` (`install.ps1`) or `--skip-attestation` (`install.sh`). Without that option it refuses to install when the GitHub CLI is missing or signed out, or when the program (or `SOURCE_COMMIT`) has no attestation from this repository's `.github/workflows/release.yml` for the release tag and commit. It also checks that the Mod it adds from the release tag is at the commit in `SOURCE_COMMIT`.

`quayflow update` checks the release's installer (`install.ps1` on Windows, `install.sh` on Linux and macOS) against (1) and runs it. So it makes the same checks as the installer, including the refusal without a signed-in GitHub CLI unless `--skip-attestation` is given. You can check everything by hand as described below.

## Check the hashes by hand

Windows, in PowerShell:

```powershell
$v = 'v0.3.0'
$base = "https://github.com/minhdaohuu/quayflow/releases/download/$v"
foreach ($f in 'quayflow.exe', 'quayflow-bg.exe', 'quayflow-linux-x86_64', 'quayflow-macos-arm64', 'quayflow-macos-x86_64', 'install.ps1', 'install.sh', 'Quayflow.psm1', 'SOURCE_COMMIT', 'SHA256SUMS') {
    try { Invoke-WebRequest -UseBasicParsing "$base/$f" -OutFile $f } catch { "$f is not in this release" }   # Linux files from v0.1.5, macOS from v0.1.6; SOURCE_COMMIT from v0.1.1
}
Get-Content SHA256SUMS | ForEach-Object {
    $hash, $name = $_ -split '\s+\*?', 2
    if (-not (Test-Path $name)) { return }
    $actual = (Get-FileHash $name -Algorithm SHA256).Hash.ToLower()
    '{0,-22} {1}' -f $name, $(if ($actual -eq $hash) { 'OK' } else { "MISMATCH ($actual)" })
}
```

Linux, in `sh`:

```sh
v=v0.3.0
base=https://github.com/minhdaohuu/quayflow/releases/download/$v
for f in quayflow-linux-x86_64 install.sh SOURCE_COMMIT SHA256SUMS; do
  curl -fsSLO "$base/$f"
done
sha256sum -c --ignore-missing SHA256SUMS
```

Every line must say `OK`. `--ignore-missing` skips the assets you did not download; in Git Bash or WSL the same `sha256sum` command checks the Windows files.

macOS (from v0.1.6), in `sh`: download `quayflow-macos-arm64` (Apple silicon) or `quayflow-macos-x86_64` (Intel) instead of the Linux program, the same way. Older macOS versions have no `sha256sum`; there, check with `shasum -a 256 -c --ignore-missing SHA256SUMS`.

A hash check proves that you have the files the release lists, but `SHA256SUMS` comes from the same release page. To tie the files to this repository's build, also check the attestation.

## Check the attestation

With the [GitHub CLI](https://cli.github.com/) signed in (`gh auth login`). Windows, in PowerShell, after the download above:

```powershell
$commit = (Get-Content SOURCE_COMMIT -Raw).Trim()
foreach ($f in 'quayflow.exe', 'quayflow-bg.exe', 'install.ps1', 'Quayflow.psm1', 'SOURCE_COMMIT', 'SHA256SUMS') {
    gh attestation verify $f --repo minhdaohuu/quayflow `
        --signer-workflow minhdaohuu/quayflow/.github/workflows/release.yml `
        --source-ref "refs/tags/$v" --source-digest $commit
}
git ls-remote https://github.com/minhdaohuu/quayflow "refs/tags/$v^{}" "refs/tags/$v"   # should list $commit
```

Linux, in `sh`, after the download above:

```sh
commit=$(tr -d '[:space:]' < SOURCE_COMMIT)
for f in quayflow-linux-x86_64 install.sh SOURCE_COMMIT SHA256SUMS; do
  gh attestation verify "$f" --repo minhdaohuu/quayflow \
    --signer-workflow minhdaohuu/quayflow/.github/workflows/release.yml \
    --source-ref "refs/tags/$v" --source-digest "$commit"
done
git ls-remote https://github.com/minhdaohuu/quayflow "refs/tags/$v^{}" "refs/tags/$v"   # should list $commit
```

On macOS, use the macOS program's name in the loop. Each must end with `Verification succeeded`. `--signer-workflow` accepts only an attestation made by the release workflow (not by any other workflow of the repository), and `--source-ref` / `--source-digest` tie it to the release tag and its commit.

**Releases before v0.1.5.** GitHub issues artifact attestations only for public repositories. v0.1.0 to v0.1.4 were built while the repository was private and are not attested. Their release notes say so, and only the hash check applies. The installer refuses them unless you pass `-SkipAttestation`. They have no Linux or macOS build.

## Rebuild it yourself

On Windows with Visual Studio Build Tools (MSVC) and rustup:

```powershell
git clone https://github.com/minhdaohuu/quayflow; cd quayflow
git checkout v0.3.0
rustup toolchain install          # installs the version pinned in rust-toolchain.toml
$env:SOURCE_DATE_EPOCH = git log -1 --format=%ct
$env:CARGO_INCREMENTAL = '0'
$src = (Get-Location).Path
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { "$env:USERPROFILE\.cargo" }
$env:CARGO_ENCODED_RUSTFLAGS = @("--remap-path-prefix=$src=.", "--remap-path-prefix=$cargoHome=/cargo", '-C', 'link-arg=/Brepro') -join [char]0x1f
cargo build --release --locked -p quayflow
Get-FileHash target\release\quayflow.exe, target\release\quayflow-bg.exe
```

The workflow proves that two builds on GitHub's Windows runner are identical. A build on your machine should match too, but differences in the MSVC linker or Windows SDK version between your machine and the runner can change the bytes. If yours differs, compare the linker version in the release job's log with yours before drawing conclusions.

On Linux with rustup, a C toolchain and the OpenSSL development files (`pkg-config`, `libssl-dev` or your distribution's equivalent):

```sh
git clone https://github.com/minhdaohuu/quayflow && cd quayflow
git checkout v0.3.0
rustup toolchain install          # installs the version pinned in rust-toolchain.toml
SOURCE_DATE_EPOCH=$(git log -1 --format=%ct); export SOURCE_DATE_EPOCH
export CARGO_INCREMENTAL=0
export RUSTFLAGS="--remap-path-prefix=$(pwd)=. --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo"
cargo build --release --locked -p quayflow --bin quayflow
sha256sum target/release/quayflow   # compare with quayflow-linux-x86_64 in SHA256SUMS
```

`RUSTFLAGS` is split at spaces, so build from a path without spaces. The workflow proves that two builds on Ubuntu 22.04 are identical. A build on another distribution or release can differ, because a different glibc, linker or C toolchain can change the bytes. To compare, build on Ubuntu 22.04 (for example in a container).

On macOS, the same steps with the Xcode Command Line Tools (no OpenSSL files needed), adding `-C link-arg=-Wl,-S` to `RUSTFLAGS` and the target: `cargo build --release --locked -p quayflow --bin quayflow --target aarch64-apple-darwin` (after `rustup target add aarch64-apple-darwin`; or `x86_64-apple-darwin`), then `shasum -a 256 target/<target>/release/quayflow`. The workflow builds both programs twice on GitHub's `macos-15` runner; a different Xcode linker version can change the bytes.

## Why releases are not code-signed

Quayflow executables are **not code-signed**: no Authenticode signature on Windows, no signature on Linux, and only the linker's ad-hoc signature on macOS (enough to run; installed with `curl`, the program carries no quarantine flag, and `install.sh` removes it from a verified copy of a browser download). This is common for open-source tools. A code-signing certificate costs money and needs a verified legal entity. The integrity properties above give a stronger link between the binary and its source: a code signature identifies who signed a file, but not what source it was built from. Signing can be added later, for example through SignPath's free program for open-source projects, without other changes.

`quayflow status` and the `/quay` pane show the build's version, its SHA-256, and how to verify it ("not code-signed; verify with the release's SHA256SUMS (and gh attestation verify when the release is attested)"). For releases before v0.1.5, which are not attested, check the hash as above.

## Windows protections and Quayflow

An unsigned executable that few people have downloaded yet can trigger the following:

**Microsoft Defender SmartScreen.** A browser download may show "Windows protected your PC" or "isn't commonly downloaded". The installer downloads with PowerShell, which does not add the Mark of the Web, so this usually does not appear. If it does appear for a file you have verified as above, choose *More info* and then *Run anyway*. Do not do this for a file you have not verified.

**Smart App Control** (Windows 11). In *On* mode, Smart App Control blocks unsigned executables that Microsoft's cloud service does not know. **It will most likely block `quayflow.exe`, and it cannot be bypassed for a single app.** Your options are to wait until Quayflow is signed, or to turn Smart App Control off. On Windows 11 24H2 and earlier, turning it off cannot be undone without reinstalling Windows. Quayflow does not recommend turning off a security feature for it.

**AppLocker and WDAC (App Control for Business).** Company machines often allow only signed software, or only software from approved paths. Quayflow runs from `%LOCALAPPDATA%`, which such policies usually block. It will not run under them, and it should not: ask your IT department. A policy can allow the file by hash, using the published `SHA256SUMS` value, but that has to be repeated for each release. Organization accounts are often used on such machines, so this is a likely place where Quayflow cannot be used yet.

**Antivirus false positives.** The engine reads login files, runs in the background and starts at sign-in if you ask it to. Heuristic scanners can flag that combination. If your antivirus flags a Quayflow file:

1. Check its hash and attestation as above. If either fails, do not use the file and please report it.
2. If both pass, it is most likely a false positive. Submit the file to your vendor, for example through the [Microsoft Defender submission portal](https://www.microsoft.com/wdsi/filesubmission), and report it in the repository so others can find it.
3. Add an exclusion only for the verified file and only if your organization allows it.

What the engine reads and writes is listed in [Security notes](security.md#what-quayflow-touches).
