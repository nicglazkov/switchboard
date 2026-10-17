#!/usr/bin/env bash
# Build the Linux x86_64 binary on the Linux host (SB_LINUX_HOST, see hosts.sh) and bring it back.
#   native/scripts/build-linux.sh
# Output: native/dist/Switchboard-linux-x86_64          plain binary (curl, chmod +x, run)
#         native/dist/Switchboard-linux-x86_64.tar.gz   binary + .desktop + icon, for a launcher entry
# Runtime needs Vulkan (mesa or vendor driver), libxkbcommon, and X11 or Wayland.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/hosts.sh
sb_hosts SB_LINUX_HOST
H=$SB_LINUX_HOST
REMOTE=${SB_REMOTE:-switchboard-native-build}

ssh -q $H "mkdir -p $REMOTE"
rsync -a --delete --exclude target --exclude dist ./ $H:$REMOTE/
ssh -q $H "set -e; export PATH=\$HOME/.cargo/bin:\$PATH; cd ~/$REMOTE
export SB_BUILD=\${SB_BUILD:-\$(date -u +%Y%m%d%H%M%S)}
start=\$(date +%s)
cargo build --release --locked -q -p sb-desktop -p sb-node
secs=\$(( \$(date +%s) - start ))
mkdir -p dist/pkg
cp target/release/Switchboard dist/Switchboard-linux-x86_64
echo \$SB_BUILD > dist/Switchboard-linux-x86_64.build
cp target/release/switchboard-node dist/switchboard-node-linux
cp target/release/Switchboard dist/pkg/Switchboard
cp packaging/linux/switchboard.desktop packaging/linux/switchboard.png dist/pkg/
tar -czf dist/Switchboard-linux-x86_64.tar.gz -C dist/pkg Switchboard switchboard.desktop switchboard.png
echo \"linux: build=\${secs}s binary=\$(stat -c %s dist/Switchboard-linux-x86_64) bytes tar.gz=\$(stat -c %s dist/Switchboard-linux-x86_64.tar.gz) bytes\""
mkdir -p dist
scp -q $H:$REMOTE/dist/Switchboard-linux-x86_64 $H:$REMOTE/dist/Switchboard-linux-x86_64.build $H:$REMOTE/dist/Switchboard-linux-x86_64.tar.gz $H:$REMOTE/dist/switchboard-node-linux dist/
chmod +x dist/Switchboard-linux-x86_64 dist/switchboard-node-linux
ls -l dist/Switchboard-linux-x86_64 dist/Switchboard-linux-x86_64.tar.gz
