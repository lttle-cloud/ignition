use anyhow::Result;
use meta::resource;

use crate::resources::{Convert, FromResource};

#[resource(name = "Certificate", tag = "certificate")]
mod certificate {
    #[version(stored + served + latest)]
    struct V1 {
        #[serde(deserialize_with = "super::de_vec_trim_non_empty_string")]
        domains: Vec<String>,
        issuer: CertificateIssuer,
    }

    #[schema]
    enum CertificateIssuer {
        #[serde(rename = "auto")]
        Auto {
            /// References a provider name from ignition.toml [[cert-provider]] config
            #[serde(deserialize_with = "super::de_trim_non_empty_string")]
            provider: String,
            /// Optional email override. If specified, takes precedence over provider's default-email.
            /// If not specified, falls back to provider's default-email from config.
            /// Validation should error if neither this nor provider config has an email.
            #[serde(default, deserialize_with = "super::de_opt_trim_non_empty_string")]
            email: Option<String>,
            /// Optional renewal configuration. Uses sensible defaults if not specified.
            renewal: Option<CertificateRenewalConfig>,
        },
        #[serde(rename = "manual")]
        Manual {
            #[serde(
                rename = "cert-path",
                deserialize_with = "super::de_trim_non_empty_string"
            )]
            cert_path: String,
            #[serde(
                rename = "key-path",
                deserialize_with = "super::de_trim_non_empty_string"
            )]
            key_path: String,
            #[serde(
                rename = "ca-path",
                default,
                deserialize_with = "super::de_opt_trim_non_empty_string"
            )]
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
        domains: Vec<String>,
        auto_provider_name: Option<String>,
    }

    #[schema]
    enum CertificateState {
        #[serde(rename = "pending")]
        Pending, // Initial state, no action taken yet

        #[serde(rename = "pending-acme-account")]
        PendingAcmeAccount, // Creating or verifying ACME account

        #[serde(rename = "pending-dns-resolution")]
        PendingDnsResolution, // Check DNS records for domains

        #[serde(rename = "pending-order")]
        PendingOrder(Option<String>), // ACME order URL (None on initial state, Some after order created)

        #[serde(rename = "pending-challenge")]
        PendingChallenge(String), // HTTP-01 challenge

        #[serde(rename = "validating")]
        Validating(String), // ACME server is validating

        #[serde(rename = "issuing")]
        Issuing(String), // Challenge passed, certificate being issued

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
    fn from_resource(resource: Certificate) -> Result<Self> {
        let certificate = resource.latest();
        Ok(CertificateStatus {
            state: CertificateState::Pending,
            not_before: None,
            not_after: None,
            last_failure_reason: None,
            renewal_time: None,
            domains: certificate.domains,
            auto_provider_name: match certificate.issuer {
                CertificateIssuer::Auto { provider, .. } => Some(provider),
                _ => None,
            },
        })
    }
}
