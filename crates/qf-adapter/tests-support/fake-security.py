#!/usr/bin/env python3
"""A stand-in for /usr/bin/security in qf-adapter's fault tests (never the real Keychain).

The Keychain is a JSON file: {"<service>": {"acct": ..., "data": <hex>, "mdat": n}}. The file
FAULTS beside it lists one fault per line, each "<command> <fault>", used once:
  find|read|write|delete  fail     exit 1 with an error on stderr
  find|read|write|delete  timeout  sleep past Quayflow's 5 s limit
  read                    hex      print the password as hex (as security does for non-ASCII)
  write                   silent   exit 0 without writing (a write that does not read back)
  write                   stderr   write, but report an error on stderr (as security -i may)
"""
import json, os, sys, time

HERE = os.path.dirname(os.path.abspath(sys.argv[0]))
STORE = os.path.join(HERE, "keychain.json")
FAULTS = os.path.join(HERE, "FAULTS")


def load():
    try:
        with open(STORE) as f:
            return json.load(f)
    except FileNotFoundError:
        return {}


def save(kc):
    with open(STORE, "w") as f:
        json.dump(kc, f)


def fault(cmd):
    try:
        lines = open(FAULTS).read().splitlines()
    except FileNotFoundError:
        return None
    for i, line in enumerate(lines):
        parts = line.split()
        if len(parts) == 2 and parts[0] == cmd:
            del lines[i]
            open(FAULTS, "w").write("\n".join(lines))
            return parts[1]
    return None


def common(cmd):
    f = fault(cmd)
    if f == "fail":
        sys.stderr.write("security: SecKeychainSearchCopyNext: simulated failure\n")
        sys.exit(1)
    if f == "timeout":
        time.sleep(8)
    return f


def opt(args, name):
    return args[args.index(name) + 1] if name in args else None


def main():
    args = sys.argv[1:]
    kc = load()
    if args[0] == "find-generic-password":
        service = opt(args, "-s")
        if "-w" in args:
            f = common("read")
            item = kc.get(service)
            if not item:
                sys.exit(44)
            data = bytes.fromhex(item["data"])
            try:
                text = data.decode("ascii")
                printable = all(32 <= ord(c) < 127 for c in text)
            except UnicodeDecodeError:
                printable = False
            print(item["data"] if f == "hex" or not printable else text)
            return
        common("find")
        item = kc.get(service)
        if not item:
            sys.exit(44)
        print('keychain: "/fake.keychain-db"')
        print("attributes:")
        print(f'    "acct"<blob>="{item["acct"]}"')
        print(f'    "mdat"<timedate>=0x00  "{item["mdat"]}Z\\000"')
        return
    if args[0] == "delete-generic-password":
        common("delete")
        service = opt(args, "-s")
        if service not in kc:
            sys.exit(44)
        del kc[service]
        save(kc)
        return
    if args[0] == "-i":
        f = common("write")
        line = sys.stdin.read().strip()
        words = line.split('"')
        # add-generic-password -U -a "<acct>" -s "<service>" -X "<hex>"
        acct, service, data = words[1], words[3], words[5]
        if f != "silent":
            prev = kc.get(service, {}).get("mdat", 20260101000000)
            kc[service] = {"acct": acct, "data": data, "mdat": prev + 1}
            save(kc)
        if f == "stderr":
            sys.stderr.write("security: SecKeychainItemModifyContent: simulated error\n")
        return
    sys.stderr.write(f"fake security: unsupported {args}\n")
    sys.exit(2)


main()
