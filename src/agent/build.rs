use anyhow::{Result, bail};
use rand::seq::IndexedRandom;

#[derive(Debug, Clone)]
pub struct BuildAgentConfig {
    pub remote_build_ca_cert_path: String,
    pub remote_build_ca_key_path: String,
    pub builders_pool: Vec<String>,
}

pub struct BuildAgent {
    pub remote_build_ca_cert_pem: String,
    pub remote_build_ca_key_pem: String,
    pub builders_pool: Vec<String>,
}

pub struct BuilderAuth {
    pub host: String,
    pub client_cert_pem: String,
    pub client_key_pem: String,
    pub ca_cert_pem: String,
}

impl BuildAgent {
    pub fn new(config: BuildAgentConfig) -> Result<Self> {
        let remote_build_ca_key_pem = std::fs::read_to_string(config.remote_build_ca_key_path)?;
        let remote_build_ca_cert_pem = std::fs::read_to_string(config.remote_build_ca_cert_path)?;
        let builders_pool = config.builders_pool;

        Ok(Self {
            remote_build_ca_cert_pem,
            remote_build_ca_key_pem,
            builders_pool,
        })
    }

    pub fn pick_and_authorize_builder(
        &self,
        tenant: impl AsRef<str>,
        user: impl AsRef<str>,
    ) -> Result<BuilderAuth> {
        let Some(builder) = self.builders_pool.choose(&mut rand::rng()) else {
            bail!("No builder found");
        };

        let builder_auth = self.generate_remote_build_auth_cert(builder, tenant, user)?;
        Ok(builder_auth)
    }

    pub fn generate_remote_build_auth_cert(
        &self,
        builder: impl AsRef<str>,
        tenant: impl AsRef<str>,
        user: impl AsRef<str>,
    ) -> Result<BuilderAuth> {
        let (client_cert_pem, client_key_pem, ca_cert_pem) =
            cert_gen::issue_client_cert_with_uri_san(
                &self.remote_build_ca_cert_pem,
                &self.remote_build_ca_key_pem,
                format!("{}@{}", tenant.as_ref(), user.as_ref()).as_str(),
                format!(
                    "spiffe://lttle.cloud/tenant/{}/user/{}",
                    tenant.as_ref(),
                    user.as_ref()
                )
                .as_str(),
                10,
            )?;

        Ok(BuilderAuth {
            host: builder.as_ref().to_string(),
            client_cert_pem,
            client_key_pem,
            ca_cert_pem,
        })
    }
}

mod cert_gen {
    use rcgen::{
        Certificate, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
        Issuer, KeyPair, KeyUsagePurpose, SanType,
    };
    use time::{Duration, OffsetDateTime};

    /// Returns (client_cert_pem, client_key_pem, ca_cert_pem)
    pub fn issue_client_cert_with_uri_san(
        ca_cert_pem: &str, // your CA (or intermediate) cert in PEM
        ca_key_pem: &str,  // matching CA private key (kept server-side)
        subject_cn: &str,  // e.g., "user-abc@tenant-xyz"
        uri_san: &str,     // e.g., "spiffe://lttle.cloud/tenant/xyz/user/abc"
        ttl_minutes: i64,  // e.g., 10
    ) -> anyhow::Result<(String, String, String)> {
        let ca_key = KeyPair::from_pem(ca_key_pem)?;
        let issuer = Issuer::from_ca_cert_pem(ca_cert_pem, &ca_key)?; // uses your existing CA cert to sign

        let client_key = KeyPair::generate()?;

        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, subject_cn);

        let now = OffsetDateTime::now_utc();
        let mut params = CertificateParams::new(Vec::<String>::new())?;
        params.distinguished_name = dn;
        params.not_before = now - Duration::minutes(1);
        params.not_after = now + Duration::minutes(ttl_minutes);
        params.is_ca = IsCa::NoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        params
            .subject_alt_names
            .push(SanType::URI(uri_san.try_into()?));

        let client_cert: Certificate = params.signed_by(&client_key, &issuer)?;
        let client_cert_pem = client_cert.pem();
        let client_key_pem = client_key.serialize_pem();
        Ok((client_cert_pem, client_key_pem, ca_cert_pem.to_string()))
    }
}
