use anyhow::Result;
use meta::resource;

use crate::resources::FromResource;

#[resource(name = "Certificate", tag = "certificate")]
mod certificate {
    #[version(stored + served + latest)]
    struct V1 {
        domains: Vec<String>,
        issuer: CertificateIssuer,
    }

    #[schema]
    enum CertificateIssuer {
        #[serde(rename = "auto")]
        Auto {
            /// References a provider name from ignition.toml [[cert-provider]] config
            provider: String,
            /// Optional email override. If specified, takes precedence over provider's default-email.
            /// If not specified, falls back to provider's default-email from config.
            /// Validation should error if neither this nor provider config has an email.
            email: Option<String>,
            /// Optional renewal configuration. Uses sensible defaults if not specified.
            renewal: Option<CertificateRenewalConfig>,
        },
        #[serde(rename = "manual")]
        Manual {
            cert_path: String,
            key_path: String,
            ca_path: Option<String>,
        },
    }

    #[schema]
    struct CertificateRenewalConfig {
        /// Days before expiry to start renewal attempts. Default: 30 days.
        #[serde(rename = "days-before-expiry")]
        days_before_expiry: Option<u32>,
        /// Hours between renewal retry attempts on failure. Default: 12 hours.
        #[serde(rename = "retry-interval-hours")]
        retry_interval_hours: Option<u32>,
    }

    #[status]
    struct Status {
        state: CertificateState,
        not_before: Option<String>,
        not_after: Option<String>,
        last_failure_reason: Option<String>,
        renewal_time: Option<String>,
    }

    #[schema]
    enum CertificateState {
        #[serde(rename = "pending")]
        Pending, // Initial state, no action taken yet

        #[serde(rename = "pending-order")]
        PendingOrder, // ACME order created, waiting for authorizations

        #[serde(rename = "pending-dns-challenge")]
        PendingDnsChallenge, // DNS-01 challenge, waiting for DNS propagation

        #[serde(rename = "pending-http-challenge")]
        PendingHttpChallenge, // HTTP-01 challenge, waiting for validation

        #[serde(rename = "validating")]
        Validating, // ACME server is validating the challenge

        #[serde(rename = "issuing")]
        Issuing, // Challenge passed, certificate being issued

        #[serde(rename = "ready")]
        Ready, // Certificate active and deployed

        #[serde(rename = "renewing")]
        Renewing, // Renewal in progress (keeps serving old cert)

        #[serde(rename = "failed")]
        Failed, // Error occurred, will retry

        #[serde(rename = "expired")]
        Expired, // Certificate has expired

        #[serde(rename = "revoked")]
        Revoked, // Certificate was revoked
    }
}

impl FromResource<Certificate> for CertificateStatus {
    fn from_resource(_resource: Certificate) -> Result<Self> {
        Ok(CertificateStatus {
            state: CertificateState::Pending,
            not_before: None,
            not_after: None,
            last_failure_reason: None,
            renewal_time: None,
        })
    }
}
