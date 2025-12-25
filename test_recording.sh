#!/bin/bash
# Test script for call recording feature
set -e

API_BASE="http://localhost:8080/v1"
SERVER_IP="${SERVER_IP:-192.168.1.81}"

echo "======================================"
echo "FCP Call Recording Test Script"
echo "======================================"
echo

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

function print_success() {
    echo -e "${GREEN}✓${NC} $1"
}

function print_error() {
    echo -e "${RED}✗${NC} $1"
}

function print_info() {
    echo -e "${YELLOW}ℹ${NC} $1"
}

# Step 1: Check if server is running
echo "Step 1: Checking if fcp-server is running..."
if curl -s "$API_BASE/health" > /dev/null 2>&1; then
    print_success "fcp-server is running"
else
    print_error "fcp-server is not running"
    echo "Please start fcp-server first:"
    echo "  ./target/release/fcp-server"
    exit 1
fi

# Step 2: Create test users
echo
echo "Step 2: Creating test users..."

# Create Alice
if curl -s -X POST "$API_BASE/users" \
    -H "Content-Type: application/json" \
    -d '{
        "username": "alice",
        "password": "secret123",
        "display_name": "Alice Test",
        "enabled": true
    }' | grep -q "alice"; then
    print_success "User alice created (or already exists)"
else
    print_info "User alice may already exist"
fi

# Create Bob
if curl -s -X POST "$API_BASE/users" \
    -H "Content-Type: application/json" \
    -d '{
        "username": "bob",
        "password": "secret456",
        "display_name": "Bob Test",
        "enabled": true
    }' | grep -q "bob"; then
    print_success "User bob created (or already exists)"
else
    print_info "User bob may already exist"
fi

# Step 3: Display SIP configuration
echo
echo "Step 3: SIP Client Configuration"
echo "======================================"
echo
echo "Configure your SIP clients with these settings:"
echo
echo "ALICE:"
echo "  Username: alice"
echo "  Password: secret123"
echo "  Domain:   $SERVER_IP"
echo "  Proxy:    $SERVER_IP:5060"
echo
echo "BOB:"
echo "  Username: bob"
echo "  Password: secret456"
echo "  Domain:   $SERVER_IP"
echo "  Proxy:    $SERVER_IP:5060"
echo

# Wait for user to register
echo "Press ENTER after both clients are registered..."
read

# Step 4: Check registrations
echo
echo "Step 4: Checking registrations..."

if curl -s "$API_BASE/registrations/sip:alice@$SERVER_IP" | grep -q "alice"; then
    print_success "Alice is registered"
else
    print_error "Alice is not registered"
fi

if curl -s "$API_BASE/registrations/sip:bob@$SERVER_IP" | grep -q "bob"; then
    print_success "Bob is registered"
else
    print_error "Bob is not registered"
fi

# Step 5: Initiate call
echo
echo "Step 5: Call Setup"
echo "======================================"
echo "From Alice's softphone, call Bob:"
echo "  Dial: sip:bob@$SERVER_IP"
echo "  or just: bob"
echo
echo "Answer the call on Bob's softphone."
echo
echo "Press ENTER after the call is established..."
read

# Step 6: Get Call ID
echo
echo "Step 6: Finding Call ID..."
CALLS=$(curl -s "$API_BASE/debug/calls")
echo "Active calls: $CALLS"

if echo "$CALLS" | grep -q "call-"; then
    CALL_ID=$(echo "$CALLS" | grep -o "call-[a-f0-9-]\+" | head -1)
    print_success "Found Call ID: $CALL_ID"
else
    print_error "No active calls found"
    echo
    print_info "Please check fcp-server logs for Call ID"
    echo "Look for lines like: 'Media session created call_id=\"call-...\"'"
    echo
    echo "Enter Call ID manually:"
    read CALL_ID
fi

# Step 7: Start recording
echo
echo "Step 7: Starting recording..."
RECORDING_PATH="/tmp/fcp_test_recording_$(date +%s).wav"

RESPONSE=$(curl -s -X POST "$API_BASE/calls/$CALL_ID/recording/start" \
    -H "Content-Type: application/json" \
    -d "{\"path\": \"$RECORDING_PATH\"}")

if echo "$RESPONSE" | grep -q "Recording started"; then
    print_success "Recording started to: $RECORDING_PATH"
else
    print_error "Failed to start recording"
    echo "Response: $RESPONSE"
    exit 1
fi

# Step 8: Record conversation
echo
echo "Step 8: Recording in progress..."
echo "======================================"
echo "Have a conversation between Alice and Bob."
echo "Recording will capture both parties."
echo
echo "Press ENTER when ready to stop recording..."
read

# Step 9: Stop recording
echo
echo "Step 9: Stopping recording..."

RESPONSE=$(curl -s -X POST "$API_BASE/calls/$CALL_ID/recording/stop")

if echo "$RESPONSE" | grep -q "Recording stopped"; then
    print_success "Recording stopped"
else
    print_error "Failed to stop recording"
    echo "Response: $RESPONSE"
fi

# Step 10: Verify recording
echo
echo "Step 10: Verifying recording..."

if [ -f "$RECORDING_PATH" ]; then
    FILE_SIZE=$(stat -f%z "$RECORDING_PATH" 2>/dev/null || stat -c%s "$RECORDING_PATH" 2>/dev/null)
    if [ "$FILE_SIZE" -gt 44 ]; then
        print_success "Recording file created: $RECORDING_PATH"
        print_success "File size: $FILE_SIZE bytes"
        echo
        echo "======================================"
        echo "✓ Test completed successfully!"
        echo "======================================"
        echo
        echo "To play the recording:"
        echo "  aplay $RECORDING_PATH"
        echo "  # or"
        echo "  ffplay $RECORDING_PATH"
        echo
        echo "To convert to MP3:"
        echo "  ffmpeg -i $RECORDING_PATH ${RECORDING_PATH%.wav}.mp3"
        echo
    else
        print_error "Recording file is too small (only $FILE_SIZE bytes)"
        print_info "The recording may be empty or corrupted"
    fi
else
    print_error "Recording file not found at: $RECORDING_PATH"
fi

echo
echo "End call from either softphone when done."
