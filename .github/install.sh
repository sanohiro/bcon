#!/bin/sh
# bcon apt repository installer
# Usage: curl -fsSL https://sanohiro.github.io/bcon/install.sh | sudo sh

set -e

# Pick the suite that matches this system's glibc.
#
#   stable - built on Ubuntu 24.04, needs glibc 2.39+
#            (Ubuntu 24.04+, Debian 13 trixie+)
#   legacy - built on Ubuntu 22.04, needs glibc 2.34+
#            (Ubuntu 22.04, Debian 12 bookworm, Raspberry Pi OS bookworm)
SUITE=stable
GLIBC=$(ldd --version 2>/dev/null | head -1 | grep -oE '[0-9]+\.[0-9]+$' || true)
if [ -n "${GLIBC}" ] && [ "$(printf '2.39\n%s\n' "${GLIBC}" | sort -V | head -1)" != "2.39" ]; then
    SUITE=legacy
fi
echo "Detected glibc ${GLIBC:-unknown}, using '${SUITE}' suite."

# Add GPG key (--yes so re-running this script does not prompt)
curl -fsSL https://sanohiro.github.io/bcon/bcon.gpg | gpg --dearmor --yes -o /usr/share/keyrings/bcon.gpg

# Add repository
echo "deb [signed-by=/usr/share/keyrings/bcon.gpg] https://sanohiro.github.io/bcon ${SUITE} main" > /etc/apt/sources.list.d/bcon.list

# Update package list
apt update

echo "Done! Run 'apt install bcon' to install."
