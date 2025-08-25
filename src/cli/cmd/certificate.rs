use anyhow::Result;
use ignition::resources::certificate::{
    CertificateIssuer, CertificateLatest, CertificateState, CertificateStatus,
};
use meta::{summary, table};

use crate::{
    client::get_api_client,
    cmd::{DeleteNamespacedArgs, GetNamespacedArgs, ListNamespacedArgs},
    config::Config,
    ui::message::{message_info, message_warn},
};

#[table]
pub struct CertificateTable {
    #[field(name = "name")]
    name: String,

    #[field(name = "namespace")]
    namespace: Option<String>,

    #[field(name = "state", cell_style = important)]
    state: String,

    #[field(name = "domains")]
    domains: String,

    #[field(name = "issuer")]
    issuer: String,
}

#[summary]
pub struct CertificateSummary {
    #[field(name = "name")]
    name: String,

    #[field(name = "namespace")]
    namespace: Option<String>,

    #[field(name = "state", cell_style = important)]
    state: String,

    #[field(name = "domains", cell_style = important)]
    domains: String,

    #[field(name = "issuer")]
    issuer: String,

    #[field(name = "provider")]
    provider: Option<String>,

    #[field(name = "email")]
    email: Option<String>,

    #[field(name = "not before")]
    not_before: Option<String>,

    #[field(name = "not after")]
    not_after: Option<String>,

    #[field(name = "renewal time")]
    renewal_time: Option<String>,

    #[field(name = "last failure reason")]
    last_failure_reason: Option<String>,
}

impl From<(CertificateLatest, CertificateStatus)> for CertificateTableRow {
    fn from((certificate, status): (CertificateLatest, CertificateStatus)) -> Self {
        let domains = certificate.domains.join(", ");

        let (issuer, _) = match &certificate.issuer {
            CertificateIssuer::Auto { provider, .. } => {
                (format!("auto ({})", provider), Some(provider.clone()))
            }
            CertificateIssuer::Manual { .. } => ("manual".to_string(), None),
        };

        let state = format_certificate_state(&status.state);

        Self {
            name: certificate.name,
            namespace: certificate.namespace,
            state,
            domains,
            issuer,
        }
    }
}

impl From<(CertificateLatest, CertificateStatus)> for CertificateSummary {
    fn from((certificate, status): (CertificateLatest, CertificateStatus)) -> Self {
        let domains = certificate.domains.join(", ");

        let (issuer, provider, email) = match &certificate.issuer {
            CertificateIssuer::Auto {
                provider, email, ..
            } => (
                format!("auto ({})", provider),
                Some(provider.clone()),
                email.clone(),
            ),
            CertificateIssuer::Manual { cert_path, .. } => {
                (format!("manual ({})", cert_path), None, None)
            }
        };

        let state = format_certificate_state(&status.state);

        Self {
            name: certificate.name,
            namespace: certificate.namespace,
            state,
            domains,
            issuer,
            provider,
            email,
            not_before: status.not_before,
            not_after: status.not_after,
            renewal_time: status.renewal_time,
            last_failure_reason: status.last_failure_reason,
        }
    }
}

fn format_certificate_state(state: &CertificateState) -> String {
    match state {
        CertificateState::Pending => "pending".to_string(),
        CertificateState::PendingAcmeAccount => "pending acme account".to_string(),
        CertificateState::PendingDnsResolution => "pending dns resolution".to_string(),
        CertificateState::PendingOrder(_) => "pending order".to_string(),
        CertificateState::PendingChallenge(_) => "pending challenge".to_string(),
        CertificateState::Validating(_) => "validating".to_string(),
        CertificateState::Issuing(_) => "issuing".to_string(),
        CertificateState::Ready => "ready".to_string(),
        CertificateState::Renewing => "renewing".to_string(),
        CertificateState::Failed => "failed".to_string(),
        CertificateState::Expired => "expired".to_string(),
        CertificateState::Revoked => "revoked".to_string(),
    }
}

pub async fn run_certificate_list(config: &Config, args: ListNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
    let certificates = api_client.certificate().list(args.into()).await?;

    let mut table = CertificateTable::new();

    for (certificate, status) in certificates {
        table.add_row(CertificateTableRow::from((certificate, status)));
    }

    table.print();

    Ok(())
}

pub async fn run_certificate_get(config: &Config, args: GetNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
    let (certificate, status) = api_client
        .certificate()
        .get(args.clone().into(), args.name)
        .await?;

    let summary = CertificateSummary::from((certificate, status));
    summary.print();

    Ok(())
}

pub async fn run_certificate_delete(config: &Config, args: DeleteNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
    if !args.confirm {
        message_warn(format!(
            "You are about to delete the certificate '{}'. This action cannot be undone. To confirm, run the command with --yes (or -y).",
            args.name
        ));
        return Ok(());
    }

    api_client
        .certificate()
        .delete(args.clone().into(), args.name.clone())
        .await?;

    message_info(format!("Certificate '{}' has been deleted.", args.name));

    Ok(())
}
