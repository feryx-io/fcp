//! Number porting (Local Number Portability) workflow.
//!
//! This module manages the process of porting phone numbers between carriers,
//! including request validation, status tracking, and carrier coordination.
//!
//! ## Port Lifecycle
//!
//! 1. **Draft** - Port request being prepared
//! 2. **Submitted** - Request submitted to losing carrier
//! 3. **Pending** - Awaiting carrier validation
//! 4. **Approved** - Carrier approved, FOC date set
//! 5. **Scheduled** - Port scheduled for specific date/time
//! 6. **InProgress** - Port cutover in progress
//! 7. **Completed** - Successfully ported
//!
//! Alternative outcomes:
//! - **Rejected** - Carrier rejected (invalid info)
//! - **Cancelled** - Customer cancelled
//! - **Failed** - Port failed during cutover

use crate::{geographic::Address, identifiers::TenantId, Error};
use anyhow::Result as AnyhowResult;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use uuid::Uuid;

/// Result type for internal porting operations.
type Result<T> = std::result::Result<T, Error>;

/// Unique identifier for a port request.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PortRequestId(String);

impl PortRequestId {
    /// Creates a new port request ID.
    pub fn new() -> Self {
        Self(format!("port-{}", Uuid::new_v4()))
    }

    /// Creates a port request ID from a UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(format!("port-{}", uuid))
    }

    /// Returns the inner string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for PortRequestId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for PortRequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for PortRequestId {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Error> {
        if s.starts_with("port-") {
            Ok(Self(s.to_string()))
        } else {
            // Try parsing as UUID and add prefix
            let uuid = Uuid::parse_str(s)
                .map_err(|e| Error::Parse(crate::ParseError::InvalidUri(e.to_string())))?;
            Ok(Self::from_uuid(uuid))
        }
    }
}

/// Status of a port request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortStatus {
    /// Port request is being prepared (not yet submitted).
    Draft,

    /// Port request submitted to losing carrier.
    Submitted,

    /// Awaiting validation/response from losing carrier.
    Pending,

    /// Approved by losing carrier with FOC date.
    Approved,

    /// Scheduled for specific date/time.
    Scheduled,

    /// Port cutover in progress.
    InProgress,

    /// Successfully completed.
    Completed,

    /// Rejected by losing carrier.
    Rejected,

    /// Cancelled by customer.
    Cancelled,

    /// Port failed during cutover.
    Failed,
}

impl PortStatus {
    /// Returns true if this is a terminal status (no further transitions).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            PortStatus::Completed
                | PortStatus::Rejected
                | PortStatus::Cancelled
                | PortStatus::Failed
        )
    }

    /// Returns true if the port can be cancelled.
    pub fn is_cancellable(&self) -> bool {
        matches!(
            self,
            PortStatus::Draft | PortStatus::Submitted | PortStatus::Pending | PortStatus::Approved
        )
    }

    /// Returns true if the port is active (not terminal or draft).
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            PortStatus::Submitted
                | PortStatus::Pending
                | PortStatus::Approved
                | PortStatus::Scheduled
                | PortStatus::InProgress
        )
    }
}

/// Reason for port rejection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortRejectionReason {
    /// Account number doesn't match carrier records.
    InvalidAccountNumber,

    /// Service address doesn't match carrier records.
    InvalidAddress,

    /// Authorization PIN incorrect or missing.
    InvalidPin,

    /// Number is not portable (e.g., non-geographic).
    NumberNotPortable,

    /// Customer name doesn't match carrier records.
    CustomerMismatch,

    /// Number already has pending port.
    DuplicatePort,

    /// Number not found in carrier's database.
    NumberNotFound,

    /// Other reason with description.
    Other(String),
}

impl PortRejectionReason {
    /// Returns a human-readable description.
    pub fn description(&self) -> String {
        match self {
            Self::InvalidAccountNumber => {
                "Account number does not match losing carrier records".to_string()
            }
            Self::InvalidAddress => {
                "Service address does not match losing carrier records".to_string()
            }
            Self::InvalidPin => "Authorization PIN is incorrect or missing".to_string(),
            Self::NumberNotPortable => {
                "Number is not portable (may be non-geographic or reserved)".to_string()
            }
            Self::CustomerMismatch => "Customer name does not match carrier records".to_string(),
            Self::DuplicatePort => "Number already has a pending port request".to_string(),
            Self::NumberNotFound => "Number not found in losing carrier database".to_string(),
            Self::Other(reason) => reason.clone(),
        }
    }
}

/// Type of port request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortType {
    /// Porting a single number.
    Single,

    /// Porting multiple numbers in a batch.
    Batch,

    /// Porting an entire range.
    Range,
}

/// Port request for transferring a phone number between carriers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortRequest {
    /// Unique identifier for this port request.
    pub id: PortRequestId,

    /// Tenant owning this port request.
    pub tenant_id: TenantId,

    /// Phone numbers to port (E.164 format).
    pub phone_numbers: Vec<String>,

    /// Type of port (single, batch, range).
    pub port_type: PortType,

    /// Current status.
    pub status: PortStatus,

    /// Rejection reason if status is Rejected.
    pub rejection_reason: Option<PortRejectionReason>,

    // Customer information for validation
    /// Customer/account name (must match carrier records).
    pub customer_name: String,

    /// Account number with losing carrier.
    pub account_number: String,

    /// Authorization PIN (if required by carrier).
    pub pin: Option<String>,

    /// Service address (must match carrier records).
    pub service_address: Address,

    /// Authorized contact name.
    pub authorized_contact_name: String,

    /// Authorized contact phone.
    pub authorized_contact_phone: String,

    /// Authorized contact email.
    pub authorized_contact_email: String,

    // Carrier information
    /// Losing carrier (porting from).
    pub losing_carrier: String,

    /// Losing carrier SPID (Service Provider ID).
    pub losing_carrier_spid: Option<String>,

    /// Winning carrier (porting to, usually us).
    pub winning_carrier: String,

    /// Winning carrier SPID.
    pub winning_carrier_spid: Option<String>,

    // Port timing
    /// Requested FOC (Firm Order Commitment) date.
    pub requested_foc_date: Option<SystemTime>,

    /// Actual FOC date confirmed by losing carrier.
    pub actual_foc_date: Option<SystemTime>,

    /// When request was submitted to carrier.
    pub submitted_at: Option<SystemTime>,

    /// When port was completed.
    pub completed_at: Option<SystemTime>,

    // Additional metadata
    /// Customer notes or special instructions.
    pub notes: Option<String>,

    /// Internal notes (not visible to customer).
    pub internal_notes: Option<String>,

    /// Custom metadata fields.
    pub metadata: HashMap<String, String>,

    /// When the port request was created.
    pub created_at: SystemTime,

    /// When the port request was last updated.
    pub updated_at: SystemTime,
}

impl PortRequest {
    /// Creates a new port request.
    pub fn new(
        tenant_id: TenantId,
        phone_numbers: Vec<String>,
        customer_name: String,
        account_number: String,
        service_address: Address,
        losing_carrier: String,
        winning_carrier: String,
    ) -> Self {
        let port_type = if phone_numbers.len() == 1 {
            PortType::Single
        } else {
            PortType::Batch
        };

        let now = SystemTime::now();

        Self {
            id: PortRequestId::new(),
            tenant_id,
            phone_numbers,
            port_type,
            status: PortStatus::Draft,
            rejection_reason: None,
            customer_name,
            account_number,
            pin: None,
            service_address,
            authorized_contact_name: String::new(),
            authorized_contact_phone: String::new(),
            authorized_contact_email: String::new(),
            losing_carrier,
            losing_carrier_spid: None,
            winning_carrier,
            winning_carrier_spid: None,
            requested_foc_date: None,
            actual_foc_date: None,
            submitted_at: None,
            completed_at: None,
            notes: None,
            internal_notes: None,
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Updates the status of this port request.
    pub fn update_status(&mut self, new_status: PortStatus) -> Result<()> {
        // Validate transition
        self.validate_status_transition(new_status)?;

        self.status = new_status;
        self.updated_at = SystemTime::now();

        // Update timestamps based on status
        match new_status {
            PortStatus::Submitted => {
                if self.submitted_at.is_none() {
                    self.submitted_at = Some(SystemTime::now());
                }
            }
            PortStatus::Completed if self.completed_at.is_none() => {
                self.completed_at = Some(SystemTime::now());
            }
            _ => {}
        }

        Ok(())
    }

    /// Validates a status transition.
    fn validate_status_transition(&self, new_status: PortStatus) -> Result<()> {
        // Can't change from terminal status
        if self.status.is_terminal() {
            return Err(Error::Config(format!(
                "Cannot change status from terminal state: {:?}",
                self.status
            )));
        }

        // Validate specific transitions
        let valid = match (self.status, new_status) {
            // From Draft
            (PortStatus::Draft, PortStatus::Submitted) => true,
            (PortStatus::Draft, PortStatus::Cancelled) => true,

            // From Submitted
            (PortStatus::Submitted, PortStatus::Pending) => true,
            (PortStatus::Submitted, PortStatus::Rejected) => true,
            (PortStatus::Submitted, PortStatus::Cancelled) => true,

            // From Pending
            (PortStatus::Pending, PortStatus::Approved) => true,
            (PortStatus::Pending, PortStatus::Rejected) => true,
            (PortStatus::Pending, PortStatus::Cancelled) => true,

            // From Approved
            (PortStatus::Approved, PortStatus::Scheduled) => true,
            (PortStatus::Approved, PortStatus::Cancelled) => true,

            // From Scheduled
            (PortStatus::Scheduled, PortStatus::InProgress) => true,
            (PortStatus::Scheduled, PortStatus::Cancelled) => true,

            // From InProgress
            (PortStatus::InProgress, PortStatus::Completed) => true,
            (PortStatus::InProgress, PortStatus::Failed) => true,

            // Same status is always valid (idempotent)
            (old, new) if old == new => true,

            // All other transitions invalid
            _ => false,
        };

        if !valid {
            return Err(Error::Config(format!(
                "Invalid status transition from {:?} to {:?}",
                self.status, new_status
            )));
        }

        Ok(())
    }

    /// Marks the port as rejected with a reason.
    pub fn reject(&mut self, reason: PortRejectionReason) -> Result<()> {
        self.update_status(PortStatus::Rejected)?;
        self.rejection_reason = Some(reason);
        Ok(())
    }

    /// Marks the port as cancelled.
    pub fn cancel(&mut self) -> Result<()> {
        if !self.status.is_cancellable() {
            return Err(Error::Config(format!(
                "Cannot cancel port in status: {:?}",
                self.status
            )));
        }
        self.update_status(PortStatus::Cancelled)
    }

    /// Sets the FOC date.
    pub fn set_foc_date(&mut self, foc_date: SystemTime) -> Result<()> {
        self.actual_foc_date = Some(foc_date);
        self.updated_at = SystemTime::now();
        Ok(())
    }

    /// Validates the port request data.
    pub fn validate(&self) -> Result<()> {
        // Validate phone numbers
        if self.phone_numbers.is_empty() {
            return Err(Error::Config(
                "Port request must include at least one phone number".to_string(),
            ));
        }

        for number in &self.phone_numbers {
            if !number.starts_with('+') {
                return Err(Error::Config(format!(
                    "Phone number must be in E.164 format: {}",
                    number
                )));
            }
        }

        // Validate customer info
        if self.customer_name.trim().is_empty() {
            return Err(Error::Config("Customer name is required".to_string()));
        }

        if self.account_number.trim().is_empty() {
            return Err(Error::Config("Account number is required".to_string()));
        }

        // Validate contact info (required for submission)
        if self.status != PortStatus::Draft {
            if self.authorized_contact_name.trim().is_empty() {
                return Err(Error::Config(
                    "Authorized contact name is required".to_string(),
                ));
            }

            if self.authorized_contact_phone.trim().is_empty() {
                return Err(Error::Config(
                    "Authorized contact phone is required".to_string(),
                ));
            }

            if self.authorized_contact_email.trim().is_empty() {
                return Err(Error::Config(
                    "Authorized contact email is required".to_string(),
                ));
            }
        }

        // Validate carriers
        if self.losing_carrier.trim().is_empty() {
            return Err(Error::Config("Losing carrier is required".to_string()));
        }

        if self.winning_carrier.trim().is_empty() {
            return Err(Error::Config("Winning carrier is required".to_string()));
        }

        Ok(())
    }
}

/// Port request manager for business logic.
pub struct PortingManager {
    store: Arc<dyn PortingStore>,
}

impl PortingManager {
    /// Creates a new porting manager.
    pub fn new(store: Arc<dyn PortingStore>) -> Self {
        Self { store }
    }

    /// Creates a new port request.
    pub async fn create_port_request(
        &self,
        tenant_id: TenantId,
        phone_numbers: Vec<String>,
        customer_name: String,
        account_number: String,
        service_address: Address,
        losing_carrier: String,
        winning_carrier: String,
    ) -> AnyhowResult<PortRequest> {
        let request = PortRequest::new(
            tenant_id,
            phone_numbers,
            customer_name,
            account_number,
            service_address,
            losing_carrier,
            winning_carrier,
        );

        request.validate()?;
        self.store.put_port_request(request.clone()).await?;

        Ok(request)
    }

    /// Gets a port request by ID.
    pub async fn get_port_request(&self, id: &PortRequestId) -> AnyhowResult<Option<PortRequest>> {
        self.store.get_port_request(id).await
    }

    /// Lists all port requests for a tenant.
    pub async fn list_port_requests(&self, tenant_id: &TenantId) -> AnyhowResult<Vec<PortRequest>> {
        self.store.list_port_requests(tenant_id).await
    }

    /// Lists port requests by status.
    pub async fn list_port_requests_by_status(
        &self,
        tenant_id: &TenantId,
        status: PortStatus,
    ) -> AnyhowResult<Vec<PortRequest>> {
        self.store
            .list_port_requests_by_status(tenant_id, status)
            .await
    }

    /// Submits a port request to the losing carrier.
    pub async fn submit_port_request(&self, id: &PortRequestId) -> AnyhowResult<PortRequest> {
        let mut request = self
            .store
            .get_port_request(id)
            .await?
            .ok_or_else(|| Error::Config(format!("Port request not found: {}", id)))?;

        // Validate before submission
        request.validate()?;

        // Update status
        request.update_status(PortStatus::Submitted)?;

        // In production, this would submit to carrier API
        // For now, just update the store
        self.store.put_port_request(request.clone()).await?;

        Ok(request)
    }

    /// Updates port request status.
    pub async fn update_port_status(
        &self,
        id: &PortRequestId,
        new_status: PortStatus,
    ) -> AnyhowResult<PortRequest> {
        let mut request = self
            .store
            .get_port_request(id)
            .await?
            .ok_or_else(|| Error::Config(format!("Port request not found: {}", id)))?;

        request.update_status(new_status)?;
        self.store.put_port_request(request.clone()).await?;

        Ok(request)
    }

    /// Rejects a port request.
    pub async fn reject_port_request(
        &self,
        id: &PortRequestId,
        reason: PortRejectionReason,
    ) -> AnyhowResult<PortRequest> {
        let mut request = self
            .store
            .get_port_request(id)
            .await?
            .ok_or_else(|| Error::Config(format!("Port request not found: {}", id)))?;

        request.reject(reason)?;
        self.store.put_port_request(request.clone()).await?;

        Ok(request)
    }

    /// Cancels a port request.
    pub async fn cancel_port_request(&self, id: &PortRequestId) -> AnyhowResult<PortRequest> {
        let mut request = self
            .store
            .get_port_request(id)
            .await?
            .ok_or_else(|| Error::Config(format!("Port request not found: {}", id)))?;

        request.cancel()?;
        self.store.put_port_request(request.clone()).await?;

        Ok(request)
    }

    /// Sets the FOC date for a port request.
    pub async fn set_foc_date(
        &self,
        id: &PortRequestId,
        foc_date: SystemTime,
    ) -> AnyhowResult<PortRequest> {
        let mut request = self
            .store
            .get_port_request(id)
            .await?
            .ok_or_else(|| Error::Config(format!("Port request not found: {}", id)))?;

        request.set_foc_date(foc_date)?;
        request.update_status(PortStatus::Scheduled)?;
        self.store.put_port_request(request.clone()).await?;

        Ok(request)
    }

    /// Deletes a port request.
    pub async fn delete_port_request(&self, id: &PortRequestId) -> AnyhowResult<()> {
        self.store.delete_port_request(id).await
    }
}

/// Storage trait for port requests.
#[async_trait::async_trait]
pub trait PortingStore: Send + Sync {
    /// Gets a port request by ID.
    async fn get_port_request(&self, id: &PortRequestId) -> AnyhowResult<Option<PortRequest>>;

    /// Lists all port requests for a tenant.
    async fn list_port_requests(&self, tenant_id: &TenantId) -> AnyhowResult<Vec<PortRequest>>;

    /// Lists port requests by status.
    async fn list_port_requests_by_status(
        &self,
        tenant_id: &TenantId,
        status: PortStatus,
    ) -> AnyhowResult<Vec<PortRequest>>;

    /// Saves a port request.
    async fn put_port_request(&self, request: PortRequest) -> AnyhowResult<()>;

    /// Deletes a port request.
    async fn delete_port_request(&self, id: &PortRequestId) -> AnyhowResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_request_id() {
        let id = PortRequestId::new();
        assert!(id.as_str().starts_with("port-"));

        let parsed: PortRequestId = id.to_string().parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn test_port_status_checks() {
        assert!(PortStatus::Completed.is_terminal());
        assert!(PortStatus::Rejected.is_terminal());
        assert!(PortStatus::Cancelled.is_terminal());
        assert!(PortStatus::Failed.is_terminal());
        assert!(!PortStatus::Draft.is_terminal());

        assert!(PortStatus::Draft.is_cancellable());
        assert!(PortStatus::Submitted.is_cancellable());
        assert!(!PortStatus::Completed.is_cancellable());

        assert!(PortStatus::Submitted.is_active());
        assert!(!PortStatus::Draft.is_active());
        assert!(!PortStatus::Completed.is_active());
    }

    #[test]
    fn test_port_request_creation() {
        let tenant_id = TenantId::new();
        let address = Address {
            street1: "123 Main St".to_string(),
            street2: None,
            city: "San Francisco".to_string(),
            state: "CA".to_string(),
            postal_code: "94105".to_string(),
            country: "US".to_string(),
        };

        let request = PortRequest::new(
            tenant_id,
            vec!["+14155551234".to_string()],
            "Acme Corp".to_string(),
            "ACC-12345".to_string(),
            address,
            "OldCarrier".to_string(),
            "NewCarrier".to_string(),
        );

        assert_eq!(request.status, PortStatus::Draft);
        assert_eq!(request.port_type, PortType::Single);
        assert_eq!(request.phone_numbers.len(), 1);
    }

    #[test]
    fn test_status_transitions() {
        let tenant_id = TenantId::new();
        let address = Address {
            street1: "123 Main St".to_string(),
            street2: None,
            city: "San Francisco".to_string(),
            state: "CA".to_string(),
            postal_code: "94105".to_string(),
            country: "US".to_string(),
        };

        let mut request = PortRequest::new(
            tenant_id,
            vec!["+14155551234".to_string()],
            "Acme Corp".to_string(),
            "ACC-12345".to_string(),
            address,
            "OldCarrier".to_string(),
            "NewCarrier".to_string(),
        );

        // Valid transition: Draft -> Submitted
        assert!(request.update_status(PortStatus::Submitted).is_ok());
        assert_eq!(request.status, PortStatus::Submitted);

        // Valid transition: Submitted -> Pending
        assert!(request.update_status(PortStatus::Pending).is_ok());

        // Invalid transition: Pending -> Completed (must go through Approved)
        assert!(request.update_status(PortStatus::Completed).is_err());

        // Valid path: Pending -> Approved -> Scheduled -> InProgress -> Completed
        assert!(request.update_status(PortStatus::Approved).is_ok());
        assert!(request.update_status(PortStatus::Scheduled).is_ok());
        assert!(request.update_status(PortStatus::InProgress).is_ok());
        assert!(request.update_status(PortStatus::Completed).is_ok());

        // Can't change from terminal status
        assert!(request.update_status(PortStatus::Draft).is_err());
    }

    #[test]
    fn test_port_rejection() {
        let tenant_id = TenantId::new();
        let address = Address {
            street1: "123 Main St".to_string(),
            street2: None,
            city: "San Francisco".to_string(),
            state: "CA".to_string(),
            postal_code: "94105".to_string(),
            country: "US".to_string(),
        };

        let mut request = PortRequest::new(
            tenant_id,
            vec!["+14155551234".to_string()],
            "Acme Corp".to_string(),
            "ACC-12345".to_string(),
            address,
            "OldCarrier".to_string(),
            "NewCarrier".to_string(),
        );

        request.update_status(PortStatus::Submitted).unwrap();
        request.update_status(PortStatus::Pending).unwrap();

        let reason = PortRejectionReason::InvalidAccountNumber;
        assert!(request.reject(reason.clone()).is_ok());
        assert_eq!(request.status, PortStatus::Rejected);
        assert_eq!(request.rejection_reason, Some(reason));
    }

    #[test]
    fn test_port_cancellation() {
        let tenant_id = TenantId::new();
        let address = Address {
            street1: "123 Main St".to_string(),
            street2: None,
            city: "San Francisco".to_string(),
            state: "CA".to_string(),
            postal_code: "94105".to_string(),
            country: "US".to_string(),
        };

        let mut request = PortRequest::new(
            tenant_id,
            vec!["+14155551234".to_string()],
            "Acme Corp".to_string(),
            "ACC-12345".to_string(),
            address,
            "OldCarrier".to_string(),
            "NewCarrier".to_string(),
        );

        // Can cancel from Draft
        assert!(request.cancel().is_ok());
        assert_eq!(request.status, PortStatus::Cancelled);

        // Can't cancel from terminal status
        assert!(request.cancel().is_err());
    }

    #[test]
    fn test_port_validation() {
        let tenant_id = TenantId::new();
        let address = Address {
            street1: "123 Main St".to_string(),
            street2: None,
            city: "San Francisco".to_string(),
            state: "CA".to_string(),
            postal_code: "94105".to_string(),
            country: "US".to_string(),
        };

        // Valid request
        let request = PortRequest::new(
            tenant_id,
            vec!["+14155551234".to_string()],
            "Acme Corp".to_string(),
            "ACC-12345".to_string(),
            address.clone(),
            "OldCarrier".to_string(),
            "NewCarrier".to_string(),
        );
        assert!(request.validate().is_ok());

        // Invalid: empty phone numbers
        let request = PortRequest::new(
            tenant_id,
            vec![],
            "Acme Corp".to_string(),
            "ACC-12345".to_string(),
            address.clone(),
            "OldCarrier".to_string(),
            "NewCarrier".to_string(),
        );
        assert!(request.validate().is_err());

        // Invalid: non-E.164 phone number
        let request = PortRequest::new(
            tenant_id,
            vec!["4155551234".to_string()],
            "Acme Corp".to_string(),
            "ACC-12345".to_string(),
            address.clone(),
            "OldCarrier".to_string(),
            "NewCarrier".to_string(),
        );
        assert!(request.validate().is_err());

        // Invalid: empty customer name
        let request = PortRequest::new(
            tenant_id,
            vec!["+14155551234".to_string()],
            "".to_string(),
            "ACC-12345".to_string(),
            address,
            "OldCarrier".to_string(),
            "NewCarrier".to_string(),
        );
        assert!(request.validate().is_err());
    }

    #[test]
    fn test_rejection_reason_descriptions() {
        assert!(!PortRejectionReason::InvalidAccountNumber
            .description()
            .is_empty());
        assert!(!PortRejectionReason::InvalidAddress.description().is_empty());
        assert_eq!(
            PortRejectionReason::Other("Custom reason".to_string()).description(),
            "Custom reason"
        );
    }
}
