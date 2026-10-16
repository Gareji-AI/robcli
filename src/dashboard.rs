use serde_json::{Value, json};

pub(crate) fn links(universe: u64, group: Option<u64>) -> Value {
    let base = format!("https://create.roblox.com/dashboard/creations/experiences/{universe}");
    let suffix = group.map(|id| format!("?groupId={id}")).unwrap_or_default();
    json!({
        "universeId": universe, "groupId": group,
        "overviewUrl": format!("{base}/overview{suffix}"),
        "settingsUrl": format!("{base}/configure{suffix}"),
        "retentionUrl": format!("{base}/analytics/retention{suffix}"),
        "credentialsUrl": format!("https://create.roblox.com/dashboard/credentials{suffix}")
    })
}
