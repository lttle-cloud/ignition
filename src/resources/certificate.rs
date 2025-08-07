#[resource(name = "Certificate", tag = "certificate")]
mod certificate {
    #[version(stored + served + latest)]
    struct V1 {
        domains: Vec<String>,
        issuer: CertificateIssuer,
        renewal: CertificateRenewalConfig,
    }

    #[schema]
    enum CertificateIssuer {
        #[serde(rename = "auto")]
        Auto {
            provider: AutoProvider,
            email: String,
        },
        #[serde(rename = "manual")]
        Manual {
            cert_path: String,
            key_path: String,
            ca_path: Option<String>,
        },
    }

    #[schema]
    enum AutoProvider {
        #[serde(rename = "letsencrypt")]
        LetsEncrypt { environment: LetsEncryptEnvironment },
        #[serde(rename = "zerossl")]
        ZeroSSL { api_key: String },
    }

    #[schema]
    enum LetsEncryptEnvironment {
        #[serde(rename = "production")]
        Production,
        #[serde(rename = "staging")]
        Staging,
    }

    #[schema]
    struct CertificateRenewalConfig {
        #[serde(rename = "days-before-expiry")]
        days_before_expiry: Option<u32>,
        #[serde(rename = "retry-interval")]
        retry_interval: Option<u32>,
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

        #[serde(rename = "pending-dns-validation")]
        PendingDnsValidation, // DNS-01 challenge, waiting for DNS propagation

        #[serde(rename = "pending-http-validation")]
        PendingHttpValidation, // HTTP-01 challenge, waiting for validation

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
