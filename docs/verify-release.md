# Verifying a Quayflow release

Every Quayflow release publishes these assets:

| Asset | What it is |
|---|---|
| `quayflow.exe` | Engine and command-line tool |
| `quayflow-bg.exe` | Windowless launcher used by the sign-in startup entry |
| `install.ps1` | Installer |
| `Quayflow.psm1` | PowerShell integration (also embedded in `quayflow.exe`) |
| `SHA256SUMS` | SHA-256 hash of each asset above, in `sha256sum` format |

Each asset is built by this repository's release workflow (`.github/workflows/release.yml`) and has three integrity properties:

1. **Published hash.** `SHA256SUMS` lists the SHA-256 of every asset. The installer and `quayflow update` refuse a file whose hash does not match.
2. **Build provenance.** A GitHub artifact attestation, signed through GitHub's Sigstore integration, records that the file was built by this repository's workflow from a specific commit and tag.
3. **Reproducible build.** The workflow builds the same commit twice, in separate jobs and directories, and fails if the two executables differ. The toolchain is pinned in `rust-toolchain.toml`, dependencies are locked with `Cargo.lock`, paths are remapped, `/Brepro` makes the PE header deterministic, and `SOURCE_DATE_EPOCH` is set from the tag's commit.

The installer checks (1) always and (2) when the GitHub CLI is installed and signed in. You can check everything by hand as described below.

## Check the hashes by hand

```powershell
$v = 'v0.1.0'
$base = "https://github.com/minhdaohuu/quayflow/releases/download/$v"
foreach ($f in 'quayflow.exe', 'quayflow-bg.exe', 'install.ps1', 'Quayflow.psm1', 'SHA256SUMS') {
    Invoke-WebRequest -UseBasicParsing "$base/$f" -OutFile $f
}
Get-Content SHA256SUMS | ForEach-Object {
    $hash, $name = $_ -split '\s+\*?', 2
    $actual = (Get-FileHash $name -Algorithm SHA256).Hash.ToLower()
    '{0,-16} {1}' -f $name, $(if ($actual -eq $hash) { 'OK' } else { "MISMATCH ($actual)" })
}
```

Every line must say `OK`. In Git Bash or WSL, `sha256sum -c SHA256SUMS` does the same.

A hash check proves that you have the files the release lists, but `SHA256SUMS` comes from the same release page. To tie the files to this repository's build, also check the attestation.

## Check the attestation

With the [GitHub CLI](https://cli.github.com/) signed in (`gh auth login`):

```powershell
foreach ($f in 'quayflow.exe', 'quayflow-bg.exe', 'install.ps1', 'Quayflow.psm1', 'SHA256SUMS') {
    gh attestation verify $f --repo minhdaohuu/quayflow
}
```

Each must end with `Verification succeeded`. To see which workflow, commit and tag built a file, add `--format json` and look at `buildDefinition`. The workflow should be `.github/workflows/release.yml` and the ref should be the release tag.

**Private repository.** GitHub issues artifact attestations only for public repositories, or for private ones on GitHub Enterprise Cloud. While Quayflow's repository is private, pilot releases are not attested. Their release notes say so, and only the hash check applies. Install those with `-SkipAttestation` when `gh` is signed in.

## Rebuild it yourself

On Windows with Visual Studio Build Tools (MSVC) and rustup:

```powershell
git clone https://github.com/minhdaohuu/quayflow; cd quayflow
git checkout v0.1.0
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

## Why releases are not code-signed

Quayflow v0.1 executables are **not Authenticode-signed**. This is common for open-source tools. A code-signing certificate costs money and needs a verified legal entity. The integrity properties above give a stronger link between the binary and its source: an Authenticode signature identifies who signed a file, but not what source it was built from. Signing can be added later, for example through SignPath's free program for open-source projects, without other changes.

`quayflow status` and the `/quay` Settings tab show the build's version, its SHA-256, and "not code-signed; provenance-attested".

## Windows protections and Quayflow

An unsigned executable that few people have downloaded yet can trigger the following:

**Microsoft Defender SmartScreen.** A browser download may show "Windows protected your PC" or "isn't commonly downloaded". The installer downloads with PowerShell, which does not add the Mark of the Web, so this usually does not appear. If it does appear for a file you have verified as above, choose *More info* and then *Run anyway*. Do not do this for a file you have not verified.

**Smart App Control** (Windows 11). In *On* mode, Smart App Control blocks unsigned executables that Microsoft's cloud service does not know. **It will most likely block `quayflow.exe`, and it cannot be bypassed for a single app.** Your options are to wait until Quayflow is signed, or to turn Smart App Control off. On Windows 11 24H2 and earlier, turning it off cannot be undone without reinstalling Windows. Quayflow does not recommend turning off a security feature for it.

**AppLocker and WDAC (App Control for Business).** Company machines often allow only signed software, or only software from approved paths. Quayflow runs from `%LOCALAPPDATA%`, which such policies usually block. It will not run under them, and it should not: ask your IT department. A policy can allow the file by hash, using the published `SHA256SUMS` value, but that has to be repeated for each release. Organization accounts are mostly used on exactly these machines, so this is the most likely place v0.1 cannot be used.

**Antivirus false positives.** The engine reads login files, runs in the background and starts at sign-in if you ask it to. Heuristic scanners can flag that combination. If your antivirus flags a Quayflow file:

1. Check its hash and attestation as above. If either fails, do not use the file and please report it.
2. If both pass, it is most likely a false positive. Submit the file to your vendor, for example through the [Microsoft Defender submission portal](https://www.microsoft.com/wdsi/filesubmission), and report it in the repository so others can find it.
3. Add an exclusion only for the verified file and only if your organization allows it.

What the engine reads and writes is listed in [Security notes](security.md#what-quayflow-touches).
