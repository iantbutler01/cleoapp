use crate::domain::push as domain_push;
use serde::Serialize;
use sqlx::PgPool;
use web_push::{
    ContentEncoding, IsahcWebPushClient, SubscriptionInfo, URL_SAFE_NO_PAD, Urgency,
    VapidSignatureBuilder, WebPushClient, WebPushMessageBuilder,
};

#[derive(Debug, Serialize)]
struct PushPayload {
    title: String,
    body: String,
    tag: String,
    data: PushPayloadData,
}

#[derive(Debug, Serialize)]
struct PushPayloadData {
    url: String,
    kind: String,
    count: usize,
}

fn build_vapid_signature(
    private_key: &str,
    subscription_info: &SubscriptionInfo,
) -> Result<web_push::VapidSignature, String> {
    if private_key.contains("BEGIN PRIVATE KEY") || private_key.contains("BEGIN EC PRIVATE KEY") {
        VapidSignatureBuilder::from_pem(private_key.as_bytes(), subscription_info)
            .map_err(|error| error.to_string())?
            .build()
            .map_err(|error| error.to_string())
    } else {
        VapidSignatureBuilder::from_base64(private_key, URL_SAFE_NO_PAD, subscription_info)
            .map_err(|error| error.to_string())?
            .build()
            .map_err(|error| error.to_string())
    }
}

async fn send_push_message(
    client: &IsahcWebPushClient,
    payload: &[u8],
    subscription: &domain_push::PushSubscriptionData,
    private_key: &str,
) -> Result<(), String> {
    let subscription_info = SubscriptionInfo::new(
        &subscription.endpoint,
        &subscription.keys.p256dh,
        &subscription.keys.auth,
    );

    let signature = build_vapid_signature(private_key, &subscription_info)?;

    let mut message = WebPushMessageBuilder::new(&subscription_info);
    message.set_payload(ContentEncoding::Aes128Gcm, payload);
    message.set_ttl(4 * 60 * 60);
    message.set_urgency(Urgency::Normal);
    message.set_vapid_signature(signature);

    client
        .send(message.build().map_err(|error| error.to_string())?)
        .await
        .map_err(|error| error.to_string())
}

pub async fn notify_new_content(
    db: &PgPool,
    user_id: i64,
    content_count: usize,
) -> Result<(), String> {
    let private_key = match std::env::var("VAPID_PRIVATE_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => {
            eprintln!(
                "[push] Missing VAPID_PRIVATE_KEY; skipping push notification for user {}",
                user_id
            );
            return Ok(());
        }
    };

    let subscriptions = domain_push::list_user_push_subscriptions(db, user_id)
        .await
        .map_err(|error| error.to_string())?;

    if subscriptions.is_empty() {
        return Ok(());
    }

    let client = IsahcWebPushClient::new().map_err(|error| error.to_string())?;

    let payload = PushPayload {
        title: "Cleo".to_string(),
        body: if content_count == 1 {
            "1 new item is ready".to_string()
        } else {
            format!("{} new items are ready", content_count)
        },
        tag: "cleo-content".to_string(),
        data: PushPayloadData {
            url: "/?view=queue".to_string(),
            kind: "content".to_string(),
            count: content_count,
        },
    };
    let payload_bytes = serde_json::to_vec(&payload).map_err(|error| error.to_string())?;

    for subscription in subscriptions {
        if let Err(error) =
            send_push_message(&client, &payload_bytes, &subscription, &private_key).await
        {
            eprintln!(
                "[push] Failed to send notification to {} for user {}: {}",
                subscription.endpoint, user_id, error
            );
        }
    }

    Ok(())
}
