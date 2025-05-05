#!/bin/bash
set -e

echo "Installing Dusty notification daemon..."

cargo build --release

mkdir -p ~/.config/systemd/user/

cp dusty.service ~/.config/systemd/user/

if [ -w /usr/local/bin ]; then
    cp target/release/dusty /usr/local/bin/
    chmod 755 /usr/local/bin/dusty
else
    sudo cp target/release/dusty /usr/local/bin/
    sudo chmod 755 /usr/local/bin/dusty
fi

systemctl --user daemon-reload
systemctl --user enable dusty.service
systemctl --user start dusty.service

echo "Installation complete! Dusty is now running."
echo "To test, run: notify-send 'Test' 'This is a test notification'"