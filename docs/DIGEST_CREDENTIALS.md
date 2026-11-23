# Digest Credentials (Cisco-Style Separate SIP Credentials)

## Overview

FCP supports **Cisco CUCM-style separate digest credentials**, providing security isolation between API/web authentication and SIP phone registration.

### Security Model

```
┌──────────────────────────────────────────────────────────┐
│                        User Account                       │
├──────────────────────────────────────────────────────────┤
│                                                           │
│  Main Password          →  API/Web/Admin Access          │
│  (bcrypt hash)              • Management Console          │
│                             • REST API                    │
│                             • Sensitive Operations        │
│                                                           │
│  Digest Password        →  SIP Phone Registration        │
│  (optional, bcrypt hash)    • SIP REGISTER               │
│  + HA1 Hash                 • Phone Authentication       │
│                             • Limited to SIP Only         │
│                                                           │
└──────────────────────────────────────────────────────────┘
```

### Benefits

1. **Security Isolation**: Compromised phone doesn't expose admin credentials
2. **Different Policies**: Complex passwords for admins, simple PINs for phones
3. **Separate Rotation**: Change SIP credentials without affecting API access
4. **Least Privilege**: Phones get SIP access only, not admin rights

## Default Behavior (Single Password)

By default, users have **one password** used for both API and SIP authentication:

```bash
# Create user with single password
curl -X POST http://localhost:8081/api/v1/users \
  -H "Content-Type: application/json" \
  -d '{
    "username": "alice",
    "password": "MySecurePassword123",
    "enabled": true,
    "tenant_id": "00000000-0000-0000-0000-000000000000"
  }'
```

**Result**:
- API authentication uses `MySecurePassword123`
- SIP authentication uses `MySecurePassword123`
- Simpler for users, single password to remember

## Cisco-Style (Separate Credentials)

For enhanced security, set a **separate digest password** for SIP:

### 1. Create User with Main Password

```bash
curl -X POST http://localhost:8081/api/v1/users \
  -H "Content-Type: application/json" \
  -d '{
    "username": "bob",
    "password": "AdminPassword123!",
    "enabled": true,
    "tenant_id": "00000000-0000-0000-0000-000000000000"
  }'
```

### 2. Set Separate Digest Password for SIP

```bash
curl -X PUT http://localhost:8081/api/v1/users/bob/digest-password \
  -H "Content-Type: application/json" \
  -d '{
    "digest_password": "SimpleSIPPass456"
  }'
```

**Result**:
- API authentication uses `AdminPassword123!` (strong, complex)
- SIP phones use `SimpleSIPPass456` (simpler, phone-friendly)
- Compromised phone ≠ compromised admin account

### 3. Configure SIP Phone

```
Username: bob
Password: SimpleSIPPass456  ← Use digest password, not main password!
Server: your-fcp-server.com
Port: 5060 (or 5061 for TLS)
```

## API Endpoints

### PUT /api/v1/users/:username/digest-password

**Set or update separate digest password for SIP authentication.**

**Request:**
```json
{
  "digest_password": "NewSIPPassword789"
}
```

**Response:**
- `204 No Content` - Success
- `400 Bad Request` - Invalid password format
- `404 Not Found` - User doesn't exist

**Example:**
```bash
curl -X PUT http://localhost:8081/api/v1/users/alice/digest-password \
  -H "Content-Type: application/json" \
  -d '{"digest_password": "phone1234"}'
```

### DELETE /api/v1/users/:username/digest-password

**Remove separate digest password and revert to using main password for SIP.**

**Response:**
- `204 No Content` - Success
- `404 Not Found` - User doesn't have separate digest password

**Example:**
```bash
curl -X DELETE http://localhost:8081/api/v1/users/alice/digest-password
```

**Note**: After deletion, the HA1 hash is cleared. The user must update their main password to restore SIP authentication, or set a new digest password.

## File-Based User Configuration

You can also configure digest passwords in `users.json`:

```json
[
  {
    "username": "admin",
    "password": "VeryStrongAdminPass123!",
    "digest_password": "SimpleSIPPass",
    "display_name": "System Administrator",
    "enabled": true
  },
  {
    "username": "user1",
    "password": "SinglePassword123",
    "display_name": "Regular User",
    "enabled": true
  }
]
```

**Note**:
- `admin` has separate credentials (Cisco-style)
- `user1` uses single password for both (simpler)
- Both are valid configurations

## Use Cases

### When to Use Single Password (Default)

✅ **Good for:**
- Small deployments (< 50 users)
- Home/lab environments
- Users without admin access
- Simplicity preferred over security isolation

### When to Use Separate Digest Credentials (Cisco-Style)

✅ **Good for:**
- Enterprise deployments
- Users with admin/API access
- Compliance requirements (PCI-DSS, HIPAA)
- High-security environments
- Legacy phones with limited password support

## Security Considerations

### 1. Password Strength

```bash
# Main Password - Can be complex (for admin users)
Password: MyC0mpl3x!Adm1nP@ss

# Digest Password - Phone-friendly (simple but unique)
Digest: phone1234
```

### 2. Credential Compromise Scenarios

| Compromised | Single Password | Cisco-Style |
|-------------|----------------|-------------|
| **SIP credentials stolen** | ❌ Full account access | ✅ SIP access only |
| **Phone lost/stolen** | ❌ Admin access compromised | ✅ Admin safe, revoke SIP cred |
| **Database leaked** | ⚠️ bcrypt + HA1 exposed | ⚠️ bcrypt + HA1 exposed |

**Both store HA1 hash** - this is a SIP protocol limitation. Use TLS for production!

### 3. Best Practices

1. **Always use TLS** for SIP over public internet
   ```toml
   [transport]
   tls_bind = "0.0.0.0:5061"
   require_tls = true
   ```

2. **Rotate digest passwords regularly**
   ```bash
   # Automated monthly rotation
   curl -X PUT http://localhost:8081/api/v1/users/bob/digest-password \
     -d '{"digest_password": "'$(generate_random_password)'"}'
   ```

3. **Monitor authentication events**
   ```bash
   grep "digest_password_set" /var/log/fcp/fcp.log
   ```

4. **Use separate credentials for admin users**
   - Admins: Separate digest password
   - Regular users: Single password OK

## Troubleshooting

### Problem: Phone won't register after setting digest password

**Solution**: Make sure you're using the **digest password** on the phone, not the main password.

```
Phone Config:
Password: [digest_password]  ← NOT main password!
```

### Problem: Deleted digest password, now phone won't register

**Solution**: The HA1 hash is cleared when you delete digest credentials. You must:

1. Update the main password (regenerates HA1), OR
2. Set a new digest password

```bash
# Option 1: Update main password
curl -X PUT http://localhost:8081/api/v1/users/alice/password \
  -d '{"password": "NewMainPassword123"}'

# Option 2: Set new digest password
curl -X PUT http://localhost:8081/api/v1/users/alice/digest-password \
  -d '{"digest_password": "NewDigestPass456"}'
```

### Problem: How do I check if a user has separate digest credentials?

**Solution**: Currently, the API doesn't expose password fields for security. Check logs:

```bash
# Look for digest password set events
grep "digest_password_set" /var/log/fcp/fcp.log | grep alice
```

## Migration Guide

### From Single Password → Cisco-Style

```bash
# Step 1: Users continue using current password for both API and SIP
# (No interruption)

# Step 2: Set digest passwords for users who need separation
for user in admin manager supervisor; do
  curl -X PUT http://localhost:8081/api/v1/users/$user/digest-password \
    -d '{"digest_password": "'$(generate_password)'"}'
done

# Step 3: Update phone configurations with new digest passwords
# Step 4: Verify phones register successfully
# Step 5: Change main passwords to be more complex (admin users)
```

### From Cisco-Style → Single Password

```bash
# Step 1: Update main password to the one you want for both API and SIP
curl -X PUT http://localhost:8081/api/v1/users/alice/password \
  -d '{"password": "NewSinglePassword123"}'

# Step 2: Delete digest password (reverts to using main password)
curl -X DELETE http://localhost:8081/api/v1/users/alice/digest-password

# Step 3: Update phone config to use main password
# (Phone must be reconfigured as HA1 hash was cleared)
```

## Technical Details

### Password Storage

```
Main Password:
  password: bcrypt(plaintext)          # For API authentication

Digest Password (if set):
  digest_password: bcrypt(plaintext)   # For verification
  sip_ha1_hash: MD5(user:realm:plaintext)  # For SIP Digest auth

If digest_password is NULL:
  sip_ha1_hash: MD5(user:realm:main_password)
```

### Database Schema

```sql
CREATE TABLE users (
  username TEXT PRIMARY KEY,
  password_hash TEXT NOT NULL,          -- Main password (bcrypt)
  digest_password TEXT,                 -- Optional SIP password (bcrypt)
  sip_ha1_hash TEXT,                    -- HA1 for SIP Digest
  -- ... other fields
);
```

### Authentication Flow

```
SIP REGISTER →
  1. Lookup user by username
  2. If digest_password is set:
       Use sip_ha1_hash (computed from digest_password)
     Else:
       Use sip_ha1_hash (computed from password)
  3. Validate Digest response using HA1 hash
  4. Accept/Reject registration

API Request →
  1. Lookup user by username
  2. Verify against password (bcrypt)
     (digest_password is ignored)
  3. Accept/Reject request
```

## References

- RFC 7616 - HTTP Digest Access Authentication
- RFC 7617 - The 'Basic' HTTP Authentication Scheme
- Cisco CUCM Digest Credentials Documentation
- FCP Architecture: `ARCHITECTURE-v0.2.md`

## Support

For questions or issues:
- GitHub Issues: https://github.com/feryx-io/fcp/issues
- Security Concerns: security@example.com
