#!/bin/sh
# bcon apt repository installer
# Usage: curl -fsSL https://sanohiro.github.io/bcon/install.sh | sudo sh

set -e

# Add GPG key
curl -fsSL https://sanohiro.github.io/bcon/bcon.gpg | gpg --dearmor -o /usr/share/keyrings/bcon.gpg

# Add repository
echo "deb [signed-by=/usr/share/keyrings/bcon.gpg] https://sanohiro.github.io/bcon stable main" > /etc/apt/sources.list.d/bcon.list

# Update package list
apt update

echo "Done! Run 'apt install bcon' to install."
