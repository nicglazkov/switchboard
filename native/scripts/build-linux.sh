#!/usr/bin/env bash
# Build the Linux x86_64 binary on the Linux host (SB_LINUX_HOST, see hosts.sh) and bring it back.
#   native/scripts/build-linux.sh
# Output: native/dist/Switchboard-linux-x86_64          plain binary (curl, chmod +x, run)
#         native/dist/Switchboard-linux-x86_64.tar.gz   binary + .desktop + icon + license notices, for a launcher entry
#         native/dist/symbols/<version>-<build>/         *.debug files for crash backtraces, not shipped
# Runtime needs Vulkan (mesa or vendor driver), libxkbcommon, and X11 or Wayland.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/hosts.sh
sb_hosts SB_LINUX_HOST
H=$SB_LINUX_HOST
sb_update_env
REMOTE=${SB_REMOTE:-switchboard-native-build}

ssh -q $H "mkdir -p $REMOTE"
rsync -a --delete --exclude target --exclude dist ./ ../branding.toml $H:$REMOTE/
ssh -q $H "set -e; export PATH=\$HOME/.cargo/bin:\$PATH; cd ~/$REMOTE
export SB_BUILD=\${SB_BUILD:-\$(date -u +%Y%m%d%H%M%S)}
export SB_UPDATE_PUBKEY='$SB_UPDATE_PUBKEY' SB_UPDATE_FEED='$SB_UPDATE_FEED'
start=\$(date +%s)
# Keep the line tables in the binary for objcopy to split off below (Cargo.toml's release profile says why).
CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO=off CARGO_PROFILE_RELEASE_STRIP=none \\
  cargo build --release --locked -q -p sb-desktop -p sb-node
secs=\$(( \$(date +%s) - start ))
VER=\$(sed -n 's/^version = \"\\(.*\\)\"/\\1/p' Cargo.toml | head -1)
SYM=dist/symbols/\$VER-\$SB_BUILD
rm -rf dist/symbols dist/pkg
mkdir -p \$SYM dist/pkg
# Symbols to SYM/<name>.debug; the shipped copy is stripped and points at it by name (gnu-debuglink).
for b in Switchboard switchboard-node; do
  objcopy --only-keep-debug target/release/\$b \$SYM/\$b.debug
  objcopy --strip-all --add-gnu-debuglink=\$SYM/\$b.debug target/release/\$b dist/\$b.stripped
done
mv dist/Switchboard.stripped dist/Switchboard-linux-x86_64
echo \$SB_BUILD > dist/Switchboard-linux-x86_64.build
mv dist/switchboard-node.stripped dist/switchboard-node-linux
cp dist/Switchboard-linux-x86_64 dist/pkg/Switchboard
cp packaging/linux/switchboard.desktop packaging/linux/switchboard.png packaging/THIRD-PARTY-NOTICES.txt dist/pkg/
tar -czf dist/Switchboard-linux-x86_64.tar.gz -C dist/pkg Switchboard switchboard.desktop switchboard.png THIRD-PARTY-NOTICES.txt
echo \"symbols: \$SYM (\$(du -sk \$SYM | cut -f1) KB)\"
echo \"linux: build=\${secs}s binary=\$(stat -c %s dist/Switchboard-linux-x86_64) bytes tar.gz=\$(stat -c %s dist/Switchboard-linux-x86_64.tar.gz) bytes\""
mkdir -p dist
scp -q $H:$REMOTE/dist/Switchboard-linux-x86_64 $H:$REMOTE/dist/Switchboard-linux-x86_64.build $H:$REMOTE/dist/Switchboard-linux-x86_64.tar.gz $H:$REMOTE/dist/switchboard-node-linux dist/
rsync -a $H:$REMOTE/dist/symbols/ dist/symbols/
chmod +x dist/Switchboard-linux-x86_64 dist/switchboard-node-linux
ls -l dist/Switchboard-linux-x86_64 dist/Switchboard-linux-x86_64.tar.gz
