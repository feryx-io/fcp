# FCP API Examples

Quick reference for common FCP API operations.

## Base URL

```
http://localhost:8081/api/v1
```

## User Management

### Create User

```bash
curl -X POST http://localhost:8081/api/v1/users \
  -H "Content-Type: application/json" \
  -d '{
    "username": "alice",
    "password": "SecurePass123!",
    "display_name": "Alice Johnson",
    "email": "alice@example.com",
    "enabled": true,
    "tenant_id": "00000000-0000-0000-0000-000000000000"
  }'
```

### List All Users

```bash
curl http://localhost:8081/api/v1/users
```

### Get User Details

```bash
curl http://localhost:8081/api/v1/users/alice
```

### Update User

```bash
curl -X PUT http://localhost:8081/api/v1/users/alice \
  -H "Content-Type: application/json" \
  -d '{
    "display_name": "Alice Smith",
    "email": "alice.smith@example.com",
    "enabled": true
  }'
```

### Update Password

```bash
curl -X PUT http://localhost:8081/api/v1/users/alice/password \
  -H "Content-Type: application/json" \
  -d '{
    "password": "NewSecurePass456!"
  }'
```

### Delete User

```bash
curl -X DELETE http://localhost:8081/api/v1/users/alice
```

## Digest Credentials (Cisco-Style)

### Set Separate SIP Password

```bash
# Set digest password for SIP phones
curl -X PUT http://localhost:8081/api/v1/users/alice/digest-password \
  -H "Content-Type: application/json" \
  -d '{
    "digest_password": "SimpleSIPPass123"
  }'
```

**After this:**
- API/Web uses main password: `SecurePass123!`
- SIP phones use digest password: `SimpleSIPPass123`

### Remove Digest Password (Revert to Single Password)

```bash
curl -X DELETE http://localhost:8081/api/v1/users/alice/digest-password
```

**After this:**
- Both API and SIP use main password
- HA1 hash is cleared (phone must re-register)

## Registration Management

### View User Registrations

```bash
curl http://localhost:8081/api/v1/registrations/alice@fcp.local
```

### Delete All Registrations for User

```bash
curl -X DELETE http://localhost:8081/api/v1/registrations/alice@fcp.local
```

### Delete Specific Registration

```bash
curl -X DELETE "http://localhost:8081/api/v1/registrations/alice@fcp.local/sip:alice@192.168.1.100:5060"
```

## Call Management

### List Active Calls

```bash
curl http://localhost:8081/api/v1/calls
```

### Originate Call (B2BUA)

```bash
curl -X POST http://localhost:8081/api/v1/calls \
  -H "Content-Type: application/json" \
  -d '{
    "from": "sip:alice@fcp.local",
    "to": "sip:bob@fcp.local"
  }'
```

### Get Call Details

```bash
curl http://localhost:8081/api/v1/calls/call-abc123
```

### Hangup Call

```bash
curl -X DELETE http://localhost:8081/api/v1/calls/call-abc123
```

## Recording

### Start Recording

```bash
curl -X POST http://localhost:8081/api/v1/calls/call-abc123/recording/start
```

### Stop Recording

```bash
curl -X POST http://localhost:8081/api/v1/calls/call-abc123/recording/stop
```

## System Status

### Health Check

```bash
curl http://localhost:8081/api/v1/health
```

### System Status

```bash
curl http://localhost:8081/api/v1/status
```

## Debug Endpoints

### System Summary

```bash
curl http://localhost:8081/api/v1/debug/summary
```

### List Active Dialogs

```bash
curl http://localhost:8081/api/v1/debug/dialogs
```

### List Active Subscriptions

```bash
curl http://localhost:8081/api/v1/debug/subscriptions
```

### List Active Calls

```bash
curl http://localhost:8081/api/v1/debug/calls
```

### View Rate Limits

```bash
curl http://localhost:8081/api/v1/debug/rate-limits
```

## Complete Workflow Example

### Scenario: Set up admin user with separate SIP credentials

```bash
# 1. Create admin user
curl -X POST http://localhost:8081/api/v1/users \
  -H "Content-Type: application/json" \
  -d '{
    "username": "admin",
    "password": "VerySecureAdminPass123!",
    "display_name": "System Administrator",
    "email": "admin@company.com",
    "enabled": true,
    "tenant_id": "00000000-0000-0000-0000-000000000000"
  }'

# 2. Set simple SIP password for phone
curl -X PUT http://localhost:8081/api/v1/users/admin/digest-password \
  -H "Content-Type: application/json" \
  -d '{
    "digest_password": "phone1234"
  }'

# 3. Configure SIP phone:
#    Username: admin
#    Password: phone1234  ← digest password, NOT main password!
#    Server: fcp.company.com

# 4. Verify registration
curl http://localhost:8081/api/v1/registrations/admin@fcp.local

# 5. Admin logs into web interface with:
#    Username: admin
#    Password: VerySecureAdminPass123!  ← main password
```

### Scenario: Rotate SIP credentials without affecting admin access

```bash
# Change SIP password only (admin can still log in to web)
curl -X PUT http://localhost:8081/api/v1/users/admin/digest-password \
  -H "Content-Type: application/json" \
  -d '{
    "digest_password": "newphone5678"
  }'

# Update phone configuration with new password
# Admin web access unchanged
```

### Scenario: Compromised phone, secure the account

```bash
# 1. Immediately delete digest credentials
curl -X DELETE http://localhost:8081/api/v1/users/admin/digest-password

# 2. Phone can no longer register (HA1 hash cleared)
# 3. Admin can still access web with main password
# 4. Set new digest password when ready
curl -X PUT http://localhost:8081/api/v1/users/admin/digest-password \
  -H "Content-Type: application/json" \
  -d '{
    "digest_password": "secure9999"
  }'
```

## Testing with jq (Pretty Output)

```bash
# List users with pretty JSON
curl -s http://localhost:8081/api/v1/users | jq '.'

# Get specific user field
curl -s http://localhost:8081/api/v1/users/alice | jq '.email'

# Count active registrations
curl -s http://localhost:8081/api/v1/registrations/alice@fcp.local | jq 'length'

# Get all usernames
curl -s http://localhost:8081/api/v1/users | jq '.[].username'
```

## Error Responses

### 400 Bad Request

```json
{
  "error": "Bad Request",
  "details": "Invalid password format"
}
```

### 404 Not Found

```json
{
  "error": "Not Found",
  "details": "User 'alice' not found"
}
```

### 409 Conflict

```json
{
  "error": "Conflict",
  "details": "User already exists"
}
```

### 500 Internal Server Error

```json
{
  "error": "Internal Server Error",
  "details": "Database connection failed"
}
```

## Authentication (Future)

When API authentication is enabled:

```bash
# With API key
curl -H "X-API-Key: your-api-key-here" \
  http://localhost:8081/api/v1/users

# With Bearer token
curl -H "Authorization: Bearer your-jwt-token" \
  http://localhost:8081/api/v1/users
```

## Batch Operations

### Create Multiple Users

```bash
# Create users.json
cat > users.json << 'EOF'
{
  "users": [
    {
      "username": "alice",
      "password": "pass1",
      "display_name": "Alice"
    },
    {
      "username": "bob",
      "password": "pass2",
      "display_name": "Bob"
    }
  ]
}
EOF

# Loop to create
for user in $(jq -r '.users[] | @base64' users.json); do
  _jq() {
    echo ${user} | base64 --decode | jq -r ${1}
  }

  curl -X POST http://localhost:8081/api/v1/users \
    -H "Content-Type: application/json" \
    -d "{
      \"username\": \"$(_jq '.username')\",
      \"password\": \"$(_jq '.password')\",
      \"display_name\": \"$(_jq '.display_name')\",
      \"enabled\": true,
      \"tenant_id\": \"00000000-0000-0000-0000-000000000000\"
    }"
done
```

## See Also

- [Digest Credentials Documentation](./DIGEST_CREDENTIALS.md)
- [API Reference](../crates/fcp-server/README.md)
- [Configuration Guide](../fcp.toml.example)
