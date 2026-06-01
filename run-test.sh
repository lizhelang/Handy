#!/bin/bash

# Script to run the newly built Handy app with existing configuration
# This preserves all user settings, models, and history

echo "Starting Handy with clipboard manager support..."
echo "Configuration directory: ~/Library/Application Support/com.pais.handy/"
echo ""

# Run the binary
./target/release/handy --debug
