# Phone Number Management API

This document describes the REST API endpoints for managing phone numbers, line bindings, extensions, and devices in FCP.

## Table of Contents

- [Phone Numbers](#phone-numbers)
- [Line Bindings](#line-bindings)
- [Extensions](#extensions)
- [Devices](#devices)
- [Authentication](#authentication)
- [Error Responses](#error-responses)

## Authentication

All API endpoints require authentication via Bearer token when `auth_enabled` is set in the configuration:

```bash
curl -H "Authorization: Bearer YOUR_TOKEN" http://localhost:8080/api/v1/phone-numbers
```

## Error Responses

All endpoints return errors in the following format:

```json
{
  "error": "Error type",
  "details": "Detailed error message"
}
```

Common HTTP status codes:
- `200 OK` - Success
- `201 Created` - Resource created successfully
- `204 No Content` - Successful deletion
- `400 Bad Request` - Invalid input
- `404 Not Found` - Resource not found
- `409 Conflict` - Resource conflict
- `500 Internal Server Error` - Server error

---

## Phone Numbers

Phone numbers are E.164-formatted numbers (e.g., `+15183217034`) that can be assigned to users and devices.

### List Phone Numbers

`GET /api/v1/phone-numbers`

Lists all phone numbers in the inventory.

**Response:**
```json
{
  "data": [
    {
      "id": "num-abc123...",
      "e164": "+15183217034",
      "country_code": "1",
      "npa": "518",
      "nxx": "321",
      "line": "7034",
      "state": "active",
      "tenant_id": "tenant-xyz...",
      "e911_address": {
        "address1": "123 Main St",
        "address2": null,
        "city": "Albany",
        "state": "NY",
        "postal_code": "12201",
        "country": "US",
        "validated": true,
        "validated_at": 1640000000
      },
      "e911_callback": "+15183217034",
      "cnam": "John Smith",
      "spam_score": null,
      "carrier": "Verizon",
      "lrn": null,
      "rate_center": "Albany",
      "acquired_at": 1640000000,
      "activated_at": 1640001000,
      "recording_enabled": false,
      "block_anonymous": false,
      "tags": ["sales", "ny-office"],
      "created_at": 1640000000,
      "updated_at": 1640000000
    }
  ],
  "next_cursor": null,
  "total": 1
}
```

### Get Phone Number

`GET /api/v1/phone-numbers/:id`

Retrieves a specific phone number by ID.

**Parameters:**
- `id` - Phone number ID (e.g., `num-abc123...`)

**Response:** Same as individual object in list response.

### Create Phone Number

`POST /api/v1/phone-numbers`

Adds a new phone number to the inventory.

**Request Body:**
```json
{
  "e164": "+15183217034",
  "tenant_id": "tenant-xyz...",
  "e911_address": {
    "address1": "123 Main St",
    "city": "Albany",
    "state": "NY",
    "postal_code": "12201",
    "country": "US"
  },
  "e911_callback": "+15183217034",
  "cnam": "John Smith",
  "carrier": "Verizon",
  "tags": ["sales"]
}
```

**Required fields:**
- `e164` - E.164 formatted phone number (must start with `+`)

**Optional fields:**
- `tenant_id` - Tenant ID (defaults to nil tenant)
- `e911_address` - E911 emergency address
- `e911_callback` - E911 callback number
- `cnam` - Caller ID name (max 15 characters)
- `carrier` - Carrier name
- `tags` - Array of tag strings

**Response:** `201 Created` with phone number object.

### Update Phone Number

`PATCH /api/v1/phone-numbers/:id`

Updates an existing phone number.

**Request Body:**
```json
{
  "state": "active",
  "e911_address": {
    "address1": "456 Oak Ave",
    "city": "Troy",
    "state": "NY",
    "postal_code": "12180",
    "country": "US"
  },
  "e911_callback": "+15183217034",
  "cnam": "Jane Doe",
  "recording_enabled": true,
  "block_anonymous": true,
  "allow_list": ["+15185551234"],
  "block_list": ["+15185559999"],
  "tags": ["support", "priority"]
}
```

**All fields are optional.**

**Response:** `200 OK` with updated phone number object.

### Delete Phone Number

`DELETE /api/v1/phone-numbers/:id`

Removes a phone number from inventory.

**Response:** `204 No Content`

### Activate Phone Number

`POST /api/v1/phone-numbers/:id/activate`

Activates a phone number (requires E911 address and callback to be set).

**Response:** `200 OK` with activated phone number object.

---

## Line Bindings

Line bindings connect phone numbers to users and devices, defining how calls should be routed.

### List Line Bindings

`GET /api/v1/line-bindings`

Lists all line bindings.

**Response:**
```json
{
  "data": [
    {
      "id": "bind-xyz123...",
      "phone_number_id": "num-abc123...",
      "owner_user_id": "user-def456...",
      "binding_type": "user",
      "devices": [
        {
          "device_id": "dev-ghi789...",
          "priority": 1,
          "line_appearance": 1,
          "shared_line": false,
          "enabled": true
        }
      ],
      "ring_strategy": "simultaneous",
      "tenant_id": "tenant-xyz...",
      "outbound_caller_id": null,
      "outbound_cnam": null,
      "created_at": 1640000000,
      "updated_at": 1640000000
    }
  ],
  "next_cursor": null,
  "total": 1
}
```

### Get Line Binding

`GET /api/v1/line-bindings/:id`

Retrieves a specific line binding.

**Response:** Same as individual object in list response.

### Create Line Binding

`POST /api/v1/line-bindings`

Creates a new line binding.

**Request Body:**
```json
{
  "phone_number_id": "num-abc123...",
  "owner_user_id": "user-def456...",
  "binding_type": "user",
  "tenant_id": "tenant-xyz...",
  "devices": [
    {
      "device_id": "dev-ghi789...",
      "priority": 1,
      "line_appearance": 1,
      "shared_line": false,
      "enabled": true
    }
  ],
  "ring_strategy": "simultaneous"
}
```

**Required fields:**
- `phone_number_id` - Phone number to bind
- `binding_type` - Type of binding (see binding types below)

**Optional fields:**
- `owner_user_id` - User who owns this line
- `tenant_id` - Tenant ID
- `devices` - Array of device bindings
- `ring_strategy` - How to ring devices (default: `simultaneous`)

Forwarding and the ring time are the owner's calling settings, not the
line's: `GET`/`PUT /api/v1/users/:username/calling` (and `/api/v1/me/calling`
for the user themself), see `docs/ENTERPRISE_CALLING.md` §3.1. They apply
to every call for the user, by number, by extension or from a flow.

**Binding Types:**
- `user` - Normal user line
- `ivr` - Auto-attendant/IVR
- `queue` - Call center queue
- `hunt_group` - Hunt group
- `conference_bridge` - Conference bridge
- `voicemail` - Direct voicemail access
- `parking` - Call parking slot
- `paging` - Overhead paging
- `fax` - Fax service
- `emergency` - Emergency callback line

**Ring Strategies:**
- `simultaneous` - Ring all devices at once
- `sequential` - Ring devices in priority order
- `round_robin` - Distribute calls evenly
- `longest_idle` - Ring device idle longest
- `primary` - Only ring primary device

**Response:** `201 Created` with line binding object.

### Update Line Binding

`PATCH /api/v1/line-bindings/:id`

Updates an existing line binding.

**Request Body:** Same as create, all fields optional.

**Response:** `200 OK` with updated line binding object.

### Delete Line Binding

`DELETE /api/v1/line-bindings/:id`

Removes a line binding.

**Response:** `204 No Content`

---

## Extensions

Extensions provide short-code dialing (e.g., "1234") that map to users, phone numbers, devices, or services.

### List Extensions

`GET /api/v1/extensions`

Lists all extensions.

**Response:**
```json
{
  "data": [
    {
      "id": "ext-abc123...",
      "number": "1234",
      "target": {
        "type": "user",
        "id": "user-def456..."
      },
      "tenant_id": "tenant-xyz...",
      "description": "Alice's extension",
      "enabled": true,
      "created_at": 1640000000,
      "updated_at": 1640000000
    }
  ],
  "next_cursor": null,
  "total": 1
}
```

### Get Extension

`GET /api/v1/extensions/:number`

Retrieves a specific extension by number.

**Response:** Same as individual object in list response.

### Create Extension

`POST /api/v1/extensions`

Creates a new extension.

**Request Body:**
```json
{
  "number": "1234",
  "target": {
    "type": "user",
    "id": "user-def456..."
  },
  "tenant_id": "tenant-xyz...",
  "description": "Alice's extension",
  "enabled": true
}
```

**Required fields:**
- `number` - Extension number (2-10 digits, cannot start with 0 or 1)
- `target` - Extension target (see target types below)

**Optional fields:**
- `tenant_id` - Tenant ID
- `description` - Human-readable description
- `enabled` - Whether extension is enabled (default: true)

**Target Types:**
```json
{ "type": "user", "id": "user-xxx..." }
{ "type": "phone_number", "id": "num-xxx..." }
{ "type": "device", "id": "dev-xxx..." }
{ "type": "service", "id": "svc-xxx..." }
```

**Response:** `201 Created` with extension object.

### Update Extension

`PATCH /api/v1/extensions/:number`

Updates an existing extension.

**Request Body:** Same as create, all fields optional except `number`.

**Response:** `200 OK` with updated extension object.

### Delete Extension

`DELETE /api/v1/extensions/:number`

Removes an extension.

**Response:** `204 No Content`

---

## Devices

Devices represent physical or soft endpoints (desk phones, softphones, WebRTC clients).

### List Devices

`GET /api/v1/devices`

Lists all devices.

**Response:**
```json
{
  "data": [
    {
      "id": "dev-abc123...",
      "name": "Alice's Desk Phone",
      "device_type": "desk_phone",
      "mac_address": "00:11:22:33:44:55",
      "model": "Cisco 8861",
      "firmware_version": "12.5.1",
      "tenant_id": "tenant-xyz...",
      "owner_user_id": "user-def456...",
      "sip_uri": "sip:alice-desk@192.168.1.100",
      "registration_state": "registered",
      "last_registration": 1640000000,
      "last_ip_address": "192.168.1.100",
      "user_agent": "Cisco/SPA8800",
      "max_lines": 6,
      "supports_video": true,
      "supports_blf": true,
      "codec_support": ["Pcmu", "Pcma", "G722"],
      "location": "Building A, Floor 2, Room 205",
      "e911_override": null,
      "provisioning_url": "https://provision.example.com/device/123",
      "config_template_id": "template-001",
      "enabled": true,
      "created_at": 1640000000,
      "updated_at": 1640000000
    }
  ],
  "next_cursor": null,
  "total": 1
}
```

### Get Device

`GET /api/v1/devices/:id`

Retrieves a specific device.

**Response:** Same as individual object in list response.

### Create Device

`POST /api/v1/devices`

Creates a new device.

**Request Body:**
```json
{
  "name": "Alice's Desk Phone",
  "device_type": "desk_phone",
  "tenant_id": "tenant-xyz...",
  "owner_user_id": "user-def456...",
  "mac_address": "00:11:22:33:44:55",
  "model": "Cisco 8861",
  "sip_uri": "sip:alice-desk@192.168.1.100",
  "max_lines": 6,
  "supports_video": true,
  "supports_blf": true,
  "location": "Building A, Floor 2, Room 205",
  "e911_override": {
    "address1": "123 Main St",
    "city": "Albany",
    "state": "NY",
    "postal_code": "12201",
    "country": "US"
  }
}
```

**Required fields:**
- `name` - Device name
- `device_type` - Type of device (see device types below)

**Optional fields:**
- `tenant_id` - Tenant ID
- `owner_user_id` - User who owns this device
- `mac_address` - MAC address for physical phones
- `model` - Device model
- `sip_uri` - SIP URI for registration
- `max_lines` - Number of line appearances (default: 1)
- `supports_video` - Video capability (default: false)
- `supports_blf` - BLF/shared line support (default: false)
- `location` - Physical location
- `e911_override` - Per-device E911 address (for mobile users)

**Device Types:**
- `desk_phone` - Physical desk phone
- `softphone` - Software phone (PC/mobile app)
- `webrtc` - WebRTC browser client
- `conference_room` - Conference room system
- `ata` - Analog telephone adapter
- `gateway` - SIP gateway
- `mobile` - Mobile device
- `other` - Other/unknown type

**Response:** `201 Created` with device object.

### Update Device

`PATCH /api/v1/devices/:id`

Updates an existing device.

**Request Body:** Same as create, all fields optional except `name` and `device_type`.

**Response:** `200 OK` with updated device object.

### Delete Device

`DELETE /api/v1/devices/:id`

Removes a device.

**Response:** `204 No Content`

---

## Complete Examples

### Example 1: Assign a Phone Number to a User with Multiple Devices

**Step 1:** Create a phone number
```bash
curl -X POST http://localhost:8080/api/v1/phone-numbers \
  -H "Content-Type: application/json" \
  -d '{
    "e164": "+15183217034",
    "e911_address": {
      "address1": "123 Main St",
      "city": "Albany",
      "state": "NY",
      "postal_code": "12201",
      "country": "US"
    },
    "e911_callback": "+15183217034",
    "cnam": "Alice Smith"
  }'
```

**Step 2:** Create devices
```bash
# Desk phone
curl -X POST http://localhost:8080/api/v1/devices \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Alice Desk Phone",
    "device_type": "desk_phone",
    "owner_user_id": "user-alice123",
    "model": "Cisco 8861"
  }'

# Softphone
curl -X POST http://localhost:8080/api/v1/devices \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Alice Softphone",
    "device_type": "softphone",
    "owner_user_id": "user-alice123",
    "model": "Zoiper 5"
  }'
```

**Step 3:** Create line binding with simultaneous ring
```bash
curl -X POST http://localhost:8080/api/v1/line-bindings \
  -H "Content-Type: application/json" \
  -d '{
    "phone_number_id": "num-abc123",
    "owner_user_id": "user-alice123",
    "binding_type": "user",
    "ring_strategy": "simultaneous",
    "devices": [
      {
        "device_id": "dev-desk001",
        "priority": 1,
        "line_appearance": 1,
        "shared_line": false,
        "enabled": true
      },
      {
        "device_id": "dev-soft001",
        "priority": 1,
        "line_appearance": 1,
        "shared_line": false,
        "enabled": true
      }
    ]
  }'
```

**Step 4:** Create extension
```bash
curl -X POST http://localhost:8080/api/v1/extensions \
  -H "Content-Type: application/json" \
  -d '{
    "number": "1234",
    "target": {
      "type": "user",
      "id": "user-alice123"
    },
    "description": "Alice - Sales"
  }'
```

### Example 2: Set Up an IVR
```bash
# Create phone number for IVR
curl -X POST http://localhost:8080/api/v1/phone-numbers \
  -H "Content-Type: application/json" \
  -d '{
    "e164": "+15185559999",
    "e911_address": {...},
    "e911_callback": "+15185559999",
    "cnam": "Main Office"
  }'

# Bind to IVR service
curl -X POST http://localhost:8080/api/v1/line-bindings \
  -H "Content-Type: application/json" \
  -d '{
    "phone_number_id": "num-ivr001",
    "binding_type": "ivr",
    "devices": []
  }'

# Create extension for internal IVR access
curl -X POST http://localhost:8080/api/v1/extensions \
  -H "Content-Type: application/json" \
  -d '{
    "number": "5000",
    "target": {
      "type": "service",
      "id": "svc-ivr001"
    },
    "description": "Main Menu IVR"
  }'
```

---

## Implementation Status

⚠️ **Note:** These API endpoints are currently scaffolded but not fully implemented. The handlers return placeholder responses with warnings that storage backends are not yet implemented.

To complete the implementation, you need to:

1. Implement the storage traits (`PhoneNumberStore`, `LineBindingStore`, `ExtensionStore`, `DeviceStore`) in your chosen storage backend (SQLite, PostgreSQL, etc.)
2. Update database schemas to include the new tables
3. Wire up the storage implementations to the API handlers

See `crates/fcp-core/src/storage.rs` for the trait definitions.
