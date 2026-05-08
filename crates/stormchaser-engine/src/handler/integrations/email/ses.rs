#[cfg(feature = "aws-ses")]
use anyhow::{Context, Result};

#[cfg(feature = "aws-ses")]
pub async fn build_ses_client(
    region: Option<String>,
    role_arn: Option<String>,
    run_id: stormchaser_model::RunId,
) -> Result<aws_sdk_ses::Client> {
    let mut config_loader = aws_config::defaults(aws_config::BehaviorVersion::v2026_01_12());
    if let Some(r) = region {
        config_loader = config_loader.region(aws_config::Region::new(r));
    }
    let config = config_loader.load().await;

    if let Some(role) = role_arn {
        let sts_client = aws_sdk_sts::Client::new(&config);
        let session_name = format!("stormchaser-ses-{}", run_id);

        let assume_role_res = sts_client
            .assume_role()
            .role_arn(role)
            .role_session_name(session_name)
            .send()
            .await?;

        let credentials = assume_role_res
            .credentials()
            .context("Missing credentials from assume_role")?;

        let assumed_credentials = aws_sdk_ses::config::Credentials::new(
            credentials.access_key_id(),
            credentials.secret_access_key(),
            Some(credentials.session_token().to_string()),
            None,
            "StsAssumedRole",
        );

        let provider = aws_sdk_ses::config::SharedCredentialsProvider::new(assumed_credentials);
        let assumed_config = aws_sdk_ses::config::Builder::from(&config)
            .credentials_provider(provider)
            .build();

        Ok(aws_sdk_ses::Client::from_conf(assumed_config))
    } else {
        Ok(aws_sdk_ses::Client::new(&config))
    }
}

#[cfg(feature = "aws-ses")]
#[allow(clippy::too_many_arguments)]
pub async fn send_email_ses(
    from: String,
    to: Vec<String>,
    cc: Option<Vec<String>>,
    bcc: Option<Vec<String>>,
    subject: String,
    body: String,
    html: bool,
    region: Option<String>,
    role_arn: Option<String>,
    configuration_set_name: Option<String>,
    run_id: stormchaser_model::RunId,
) -> Result<()> {
    use aws_sdk_ses::types::{Body, Content, Destination, Message};

    let client = build_ses_client(region, role_arn, run_id).await?;

    let mut dest_builder = Destination::builder();
    for addr in to {
        dest_builder = dest_builder.to_addresses(addr);
    }
    if let Some(ccs) = cc {
        for addr in ccs {
            dest_builder = dest_builder.cc_addresses(addr);
        }
    }
    if let Some(bccs) = bcc {
        for addr in bccs {
            dest_builder = dest_builder.bcc_addresses(addr);
        }
    }
    let destination = dest_builder.build();

    let content = Content::builder().data(body).charset("UTF-8").build()?;
    let mut body_builder = Body::builder();
    if html {
        body_builder = body_builder.html(content);
    } else {
        body_builder = body_builder.text(content);
    }

    let message = Message::builder()
        .subject(Content::builder().data(subject).charset("UTF-8").build()?)
        .body(body_builder.build())
        .build();

    let mut request = client
        .send_email()
        .source(from)
        .destination(destination)
        .message(message);

    if let Some(cs) = configuration_set_name {
        request = request.configuration_set_name(cs);
    }

    request.send().await?;

    Ok(())
}
