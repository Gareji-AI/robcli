use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::error::AppError;

const MAX_BYTES: u64 = 1024 * 1024;

pub(crate) enum RenderFormat {
    Markdown,
    Csv,
}

pub(crate) fn compare_files(before: &Path, after: &Path) -> Result<Value, AppError> {
    compare_values(&parse_file(before)?, &parse_file(after)?)
}

pub(crate) fn render_file(
    input: &Path,
    output: &Path,
    format: RenderFormat,
) -> Result<Value, AppError> {
    let bytes = read_limited(input)?;
    let value = parse_bytes(&bytes)?;
    let body = render_value(&value, &format)?;
    write_new(output, body.as_bytes())?;
    Ok(json!({
        "inputCommand": value["command"],
        "cliVersion": value["cliVersion"],
        "format": match format { RenderFormat::Markdown => "markdown", RenderFormat::Csv => "csv" },
        "output": output.to_string_lossy(),
        "bytes": body.len(),
        "inputModified": false,
        "networkUsed": false,
        "dashboardCsvExport": false,
        "rightsVerified": false,
        "completeCrawl": false
    }))
}

pub(crate) fn compare_values(before: &Value, after: &Value) -> Result<Value, AppError> {
    let left = envelope(before)?;
    let right = envelope(after)?;
    if left.command != right.command {
        return Err(incompatible("The two snapshots are different commands."));
    }
    if left.version != right.version {
        return Err(version_mismatch());
    }
    let diff = match left.command.as_str() {
        "charts games" => compare_charts(&left.result, &right.result)?,
        "games get" => compare_game(&left.result, &right.result)?,
        "games passes" | "games products" => compare_commerce(&left.result, &right.result)?,
        _ => return Err(unsupported()),
    };
    Ok(json!({
        "command": left.command,
        "cliVersion": left.version,
        "beforeObservedAtUnixMs": left.observed,
        "afterObservedAtUnixMs": right.observed,
        "diff": diff,
        "networkUsed": false
    }))
}

fn render_value(value: &Value, format: &RenderFormat) -> Result<String, AppError> {
    let snap = envelope(value)?;
    let note = "Untrusted catalog or page text. This is not a rights check, a complete crawl, or a repaired Dashboard CSV export.";
    let (header, rows) = match snap.command.as_str() {
        "charts games" => chart_table(&snap.result)?,
        "games get" => game_table(&snap.result)?,
        "games passes" | "games products" => commerce_table(&snap.result)?,
        "store search" => store_table(&snap.store_items()?, false)?,
        "store get" => store_table(&snap.store_one()?, true)?,
        _ => return Err(unsupported()),
    };
    let observed = snap.observed.to_string();
    match format {
        RenderFormat::Markdown => Ok(format!(
            "# {command}\n\ncliVersion: {version}\n\nobservedAtUnixMs: {observed}\n\n{note}\n\n{filters}\n\n{table}\n",
            command = snap.command,
            version = snap.version,
            filters = md_cell(&snap.filter_line()),
            table = markdown(&header, &rows)
        )),
        RenderFormat::Csv => {
            let mut lines = vec![csv_row(&[
                "command",
                "cliVersion",
                "observedAtUnixMs",
                "note",
            ])];
            lines.push(csv_row(&[
                snap.command.as_str(),
                snap.version.as_str(),
                observed.as_str(),
                note,
            ]));
            lines.push(String::new());
            let header_refs: Vec<&str> = header.iter().map(String::as_str).collect();
            lines.push(csv_row(&header_refs));
            for row in rows {
                lines.push(csv_row(&row.iter().map(String::as_str).collect::<Vec<_>>()));
            }
            Ok(lines.join("\n") + "\n")
        }
    }
}

struct Snap {
    command: String,
    version: String,
    observed: u64,
    result: Value,
    data: Value,
}

impl Snap {
    fn store_items(&self) -> Result<Vec<Value>, AppError> {
        self.data["collection"]["assets"]
            .as_array()
            .cloned()
            .ok_or_else(invalid)
    }
    fn store_one(&self) -> Result<Vec<Value>, AppError> {
        if self.data["asset"].is_object() {
            Ok(vec![self.data["asset"].clone()])
        } else {
            Err(invalid())
        }
    }
    fn filter_line(&self) -> String {
        match self.command.as_str() {
            "charts games" => format!(
                "sort={sort} country={country} device={device} requestedLimit={limit} requestedPageToken={token} globalRankVerified=false",
                sort = show(&self.result["sortId"]),
                country = show(&self.result["country"]),
                device = show(&self.result["device"]),
                limit = show(&self.result["requestedLimit"]),
                token = show(&self.result["requestedPageToken"])
            ),
            "games passes" | "games products" => format!(
                "universeId={id} requestedLimit={limit} requestedPageToken={token} currency=Robux",
                id = show(&self.result["universeId"]),
                limit = show(&self.result["requestedLimit"]),
                token = show(&self.result["requestedPageToken"])
            ),
            "games get" => format!("universeId={}", show(&self.result["universeId"])),
            "store search" => format!(
                "category={} currency=USD-cents-when-present",
                show(&self.data["collection"]["category"])
            ),
            _ => "typed IDs are in the table".to_owned(),
        }
    }
}

fn envelope(value: &Value) -> Result<Snap, AppError> {
    if value["ok"] != true || value.get("error").is_some() {
        return Err(AppError::new(
            "snapshot_not_success",
            "Only a successful rob JSON envelope can be compared or rendered.",
            "Save stdout from a command that exited 0. Failure envelopes are rejected without echoing their body.",
        ));
    }
    let command = value["command"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 64)
        .ok_or_else(metadata)?
        .to_owned();
    let version = value["cliVersion"].as_str().filter(|s| version_ok(s));
    let Some(version) = version else {
        return Err(metadata());
    };
    let data = value
        .get("data")
        .filter(|v| v.is_object())
        .ok_or_else(metadata)?;
    let observed = data["observedAtUnixMs"].as_u64().ok_or_else(metadata)?;
    let result = if command.starts_with("store ") {
        Value::Null
    } else {
        data.get("result")
            .cloned()
            .filter(|v| v.is_object())
            .ok_or_else(metadata)?
    };
    Ok(Snap {
        command,
        version: version.to_owned(),
        observed,
        result,
        data: data.clone(),
    })
}

fn compare_charts(before: &Value, after: &Value) -> Result<Value, AppError> {
    require_equal(
        before,
        after,
        &[
            "sortId",
            "country",
            "device",
            "requestedPageToken",
            "requestedLimit",
        ],
    )?;
    if before["globalRankVerified"] != false || after["globalRankVerified"] != false {
        return Err(incompatible(
            "Chart snapshots must not claim a verified global rank.",
        ));
    }
    let left = index_items(before, "universeId")?;
    let right = index_items(after, "universeId")?;
    let mut entered = Vec::new();
    let mut left_page = Vec::new();
    let mut moved = Vec::new();
    let mut unchanged = Vec::new();
    for (id, row) in &left {
        match right.get(id) {
            None => left_page.push(json!({
                "universeId": id, "positionInPage": row["positionInPage"],
                "interpretation": "Absent from this fetched page range. Not proof of removal, delisting, or a drop in global rank."
            })),
            Some(other) => {
                let a = req_u64(row, "positionInPage")?;
                let b = req_u64(other, "positionInPage")?;
                if a == b {
                    unchanged.push(json!({"universeId": id, "positionInPage": a}));
                } else {
                    moved.push(json!({"universeId": id, "beforePositionInPage": a, "afterPositionInPage": b}));
                }
            }
        }
    }
    for (id, row) in &right {
        if !left.contains_key(id) {
            entered.push(json!({
                "universeId": id, "positionInPage": row["positionInPage"],
                "interpretation": "Present only in the later fetched page."
            }));
        }
    }
    Ok(json!({
        "kind": "chart-page",
        "globalRankVerified": false,
        "entered": entered,
        "left": left_page,
        "moved": moved,
        "unchanged": unchanged
    }))
}

fn compare_game(before: &Value, after: &Value) -> Result<Value, AppError> {
    let id = req_u64(before, "universeId")?;
    if req_u64(after, "universeId")? != id {
        return Err(incompatible(
            "Experience snapshots are for different Universe IDs.",
        ));
    }
    Ok(json!({
        "kind": "experience-snapshot",
        "universeId": id,
        "playing": pair_u64(before, after, "playing")?,
        "visits": pair_u64(before, after, "visits")?,
        "favorites": pair_u64(before, after, "favorites")?,
        "upVotes": pair_u64(before, after, "upVotes")?,
        "downVotes": pair_u64(before, after, "downVotes")?,
        "totalVotes": pair_u64(before, after, "totalVotes")?,
        "likePercent": pair_percent(before, after)?,
        "playingDeltaIsNotDau": true,
        "playingDeltaIsNotGrowthRate": true
    }))
}

fn compare_commerce(before: &Value, after: &Value) -> Result<Value, AppError> {
    require_equal(
        before,
        after,
        &["universeId", "requestedPageToken", "requestedLimit"],
    )?;
    let key = if before["items"][0]["passId"].is_number() || after["items"][0]["passId"].is_number()
    {
        "passId"
    } else {
        "productId"
    };
    let left = index_items(before, key)?;
    let right = index_items(after, key)?;
    let mut rows = Vec::new();
    for (id, row) in &left {
        match right.get(id) {
            None => rows.push(json!({
                "id": id, "idKind": key, "presence": "absent_from_later_page",
                "interpretation": "Absent from this fetched page range. Not proof of deletion or the end of sale."
            })),
            Some(other) => rows.push(price_row(id, key, row, other)?),
        }
    }
    for id in right.keys() {
        if !left.contains_key(id) {
            rows.push(json!({
                "id": id, "idKind": key, "presence": "present_only_in_later_page",
                "interpretation": "Present only in the later fetched page."
            }));
        }
    }
    Ok(json!({
        "kind": "commerce-page",
        "universeId": before["universeId"],
        "currency": "Robux",
        "unknownPriceIsNotFree": true,
        "rows": rows
    }))
}

fn price_row(id: &u64, key: &str, before: &Value, after: &Value) -> Result<Value, AppError> {
    let left = opt_u64(before, "priceInRobux")?;
    let right = opt_u64(after, "priceInRobux")?;
    let changed = match (left, right) {
        (Some(a), Some(b)) => Value::Bool(a != b),
        _ => Value::Null,
    };
    Ok(json!({
        "id": id, "idKind": key, "presence": "both_pages",
        "beforePriceInRobux": left, "afterPriceInRobux": right, "priceChanged": changed,
        "beforeForSale": opt_bool(before, "isForSale")?, "afterForSale": opt_bool(after, "isForSale")?,
        "unknownPriceIsNotFree": true
    }))
}

fn chart_table(result: &Value) -> Result<(Vec<String>, Vec<Vec<String>>), AppError> {
    let mut rows = Vec::new();
    for item in items(result)? {
        rows.push(vec![
            show(&item["positionInPage"]),
            show(&item["universeId"]),
            show(&item["name"]),
            show(&item["playerCount"]),
            show(&item["gameUrl"]),
        ]);
    }
    Ok((
        [
            "positionInPage",
            "universeId",
            "name",
            "playerCount",
            "gameUrl",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        rows,
    ))
}

fn game_table(result: &Value) -> Result<(Vec<String>, Vec<Vec<String>>), AppError> {
    Ok((
        ["field", "value"].into_iter().map(str::to_owned).collect(),
        [
            "universeId",
            "name",
            "playing",
            "visits",
            "favorites",
            "upVotes",
            "downVotes",
            "totalVotes",
            "likePercent",
            "gameUrl",
        ]
        .into_iter()
        .map(|key| vec![key.to_owned(), show(&result[key])])
        .collect(),
    ))
}

fn commerce_table(result: &Value) -> Result<(Vec<String>, Vec<Vec<String>>), AppError> {
    let mut rows = Vec::new();
    for item in items(result)? {
        let (kind, id) = if item["passId"].is_number() {
            ("passId", show(&item["passId"]))
        } else {
            ("productId", show(&item["productId"]))
        };
        rows.push(vec![
            kind.to_owned(),
            id,
            show(&item["name"]),
            robux(&item["priceInRobux"]),
            show(&item["isForSale"]),
        ]);
    }
    Ok((
        ["idKind", "id", "name", "priceInRobux", "isForSale"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        rows,
    ))
}

fn store_table(
    items: &[Value],
    _detail: bool,
) -> Result<(Vec<String>, Vec<Vec<String>>), AppError> {
    let mut rows = Vec::new();
    for item in items {
        rows.push(vec![
            show(&item["asset"]["id"]),
            show(&item["asset"]["name"]),
            show(&item["asset"]["audioType"]),
            usd(&item["product"]),
            show(&item["studioAssetId"]),
        ]);
    }
    Ok((
        ["assetId", "name", "audioType", "price", "studioAssetId"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        rows,
    ))
}

fn items(result: &Value) -> Result<&Vec<Value>, AppError> {
    result["items"]
        .as_array()
        .filter(|rows| rows.len() <= 30)
        .ok_or_else(invalid)
}

fn index_items(result: &Value, key: &str) -> Result<BTreeMap<u64, Value>, AppError> {
    let mut out = BTreeMap::new();
    for item in items(result)? {
        let id = req_u64(item, key)?;
        if out.insert(id, item.clone()).is_some() {
            return Err(invalid());
        }
    }
    Ok(out)
}

fn require_equal(before: &Value, after: &Value, keys: &[&str]) -> Result<(), AppError> {
    if keys
        .iter()
        .any(|key| match (before.get(*key), after.get(*key)) {
            (Some(left), Some(right)) => left != right,
            _ => true,
        })
    {
        return Err(incompatible(
            "Country, device, sort, Universe, or requested page range does not match. Missing page metadata is not inferred.",
        ));
    }
    Ok(())
}

fn pair_u64(before: &Value, after: &Value, key: &str) -> Result<Value, AppError> {
    let left = opt_u64(before, key)?;
    let right = opt_u64(after, key)?;
    let delta = match (left, right) {
        (Some(before_count), Some(after_count)) => i64::try_from(after_count)
            .ok()
            .zip(i64::try_from(before_count).ok())
            .map(|(after_count, before_count)| after_count - before_count)
            .map(Value::from)
            .unwrap_or(Value::Null),
        _ => Value::Null,
    };
    Ok(json!({"before": left, "after": right, "delta": delta}))
}

fn pair_percent(before: &Value, after: &Value) -> Result<Value, AppError> {
    let left = opt_f64(before, "likePercent")?;
    let right = opt_f64(after, "likePercent")?;
    let points = left
        .zip(right)
        .map(|(a, b)| ((b - a) * 100.0).round() / 100.0);
    Ok(json!({"before": left, "after": right, "percentagePoints": points}))
}

fn opt_u64(value: &Value, key: &str) -> Result<Option<u64>, AppError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n.as_u64().map(Some).ok_or_else(invalid),
        _ => Err(invalid()),
    }
}
fn req_u64(value: &Value, key: &str) -> Result<u64, AppError> {
    opt_u64(value, key)?.ok_or_else(invalid)
}
fn opt_bool(value: &Value, key: &str) -> Result<Option<bool>, AppError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(v)) => Ok(Some(*v)),
        _ => Err(invalid()),
    }
}
fn opt_f64(value: &Value, key: &str) -> Result<Option<f64>, AppError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n.as_f64().map(Some).ok_or_else(invalid),
        _ => Err(invalid()),
    }
}

fn show(value: &Value) -> String {
    match value {
        Value::Null | Value::Object(_) | Value::Array(_) => "unknown".to_owned(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}
fn robux(value: &Value) -> String {
    match value.as_u64() {
        Some(n) => format!("{n} Robux"),
        None => "unknown".to_owned(),
    }
}
fn usd(product: &Value) -> String {
    let price = &product["purchasePrice"];
    match (
        price["currencyCode"].as_str(),
        price["quantity"]["significand"].as_i64(),
        price["quantity"]["exponent"].as_i64(),
    ) {
        (Some("USD"), Some(sig), Some(exp)) => format!("USD significand={sig} exponent={exp}"),
        (Some(code), _, _) => format!("{code} unknown"),
        _ => "unknown".to_owned(),
    }
}
fn markdown(header: &[String], rows: &[Vec<String>]) -> String {
    let head = header
        .iter()
        .map(|c| md_cell(c))
        .collect::<Vec<_>>()
        .join(" | ");
    let rule = header.iter().map(|_| "---").collect::<Vec<_>>().join(" | ");
    let body = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| md_cell(c))
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("| {head} |\n| {rule} |\n{body}")
}
fn md_cell(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\n' | '\r' => out.push(' '),
            '`' => out.push('\''),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '|' | '[' | ']' | '!' | '\\' | '*' | '_' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}
fn csv_row(cells: &[&str]) -> String {
    cells
        .iter()
        .map(|cell| csv_cell(cell))
        .collect::<Vec<_>>()
        .join(",")
}
fn csv_cell(value: &str) -> String {
    let mut text = value.replace(['\n', '\r'], " ");
    if text.trim_start().starts_with(['=', '+', '-', '@', '\t']) {
        text.insert(0, '\'');
    }
    if text.contains([',', '"']) || text.starts_with('\'') {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

fn parse_file(path: &Path) -> Result<Value, AppError> {
    parse_bytes(&read_limited(path)?)
}
fn read_limited(path: &Path) -> Result<Vec<u8>, AppError> {
    let file = File::open(path).map_err(|_| unreadable())?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unreadable())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(AppError::new(
            "snapshot_too_large",
            "The snapshot is larger than 1 MiB.",
            "Compare or render a single saved command envelope, not a concatenated log.",
        ));
    }
    Ok(bytes)
}
fn parse_bytes(bytes: &[u8]) -> Result<Value, AppError> {
    serde_json::from_slice(bytes).map_err(|_| unreadable())
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| {
            AppError::new(
                "report_output_exists",
                "The report output already exists or cannot be created.",
                "Choose a new file path. Existing files and the input snapshot are left unchanged.",
            )
        })?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| {
            let _ = std::fs::remove_file(path);
            AppError::new(
                "report_write_failed",
                "The report file could not be written.",
                "No second attempt was made. The input snapshot was not modified.",
            )
        })
}
fn version_ok(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}
fn metadata() -> AppError {
    AppError::new(
        "snapshot_metadata_missing",
        "The snapshot is missing command, cliVersion, observedAtUnixMs, or result metadata.",
        "Older envelopes may not record cliVersion or requested page metadata. Those values are not inferred or backfilled.",
    )
}
fn version_mismatch() -> AppError {
    AppError::new(
        "snapshot_version_mismatch",
        "The snapshots were produced by different robcli versions.",
        "Compare two envelopes with the same cliVersion. Schema differences are not guessed.",
    )
}
fn incompatible(message: &'static str) -> AppError {
    AppError::new(
        "snapshot_incompatible",
        message,
        "Use the same command, typed target, filters, and requested page range. Partial pages are not treated as a global ranking.",
    )
}
fn unsupported() -> AppError {
    AppError::new(
        "snapshot_command_unsupported",
        "This saved command cannot be compared or rendered.",
        "Compare supports charts games, games get, games passes, and games products. Render also supports store search and store get.",
    )
}
fn invalid() -> AppError {
    AppError::new(
        "snapshot_invalid",
        "The snapshot shape is incomplete, duplicated, or not a typed ID list.",
        "Do not repair the file by guessing IDs, prices, vote rates, or page tokens.",
    )
}
fn unreadable() -> AppError {
    AppError::new(
        "snapshot_unreadable",
        "The snapshot could not be read as one JSON envelope.",
        "Pass a saved rob stdout file. The file body is not copied into the error.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(command: &str, result: Value) -> Value {
        json!({
            "ok": true, "command": command, "cliVersion": "0.7.0",
            "data": {"observedAtUnixMs": 10, "result": result}
        })
    }

    #[test]
    fn charts_ignore_array_order_and_do_not_claim_global_rank() {
        let item = |id, pos| json!({"universeId": id, "positionInPage": pos, "name": "A"});
        let before = env(
            "charts games",
            json!({
                "sortId": "top", "country": "all", "device": "computer", "requestedPageToken": null,
                "requestedLimit": 10, "globalRankVerified": false,
                "items": [item(2, 2), item(1, 1)]
            }),
        );
        let after = env(
            "charts games",
            json!({
                "sortId": "top", "country": "all", "device": "computer", "requestedPageToken": null,
                "requestedLimit": 10, "globalRankVerified": false,
                "items": [item(1, 1), item(3, 2)]
            }),
        );
        let diff = compare_values(&before, &after).unwrap();
        assert_eq!(diff["diff"]["globalRankVerified"], false);
        assert_eq!(diff["diff"]["unchanged"][0]["universeId"], 1);
        assert_eq!(diff["diff"]["left"][0]["universeId"], 2);
        assert!(
            diff["diff"]["left"][0]["interpretation"]
                .as_str()
                .unwrap()
                .contains("Not proof")
        );
        assert_eq!(diff["diff"]["entered"][0]["universeId"], 3);
    }

    #[test]
    fn null_votes_and_prices_stay_unknown() {
        let game = |votes: Option<u64>| json!({"universeId": 77, "playing": 3, "visits": 4, "favorites": 5, "upVotes": votes, "downVotes": votes, "totalVotes": votes, "likePercent": votes.map(|_| 50.0)});
        let diff = compare_values(
            &env("games get", game(Some(1))),
            &env("games get", game(None)),
        )
        .unwrap();
        assert!(diff["diff"]["likePercent"]["percentagePoints"].is_null());
        assert!(diff["diff"]["upVotes"]["delta"].is_null());
        assert_eq!(diff["diff"]["playing"]["delta"], 0);
        let row = json!({"productId": 5, "name": "Gem", "priceInRobux": null, "isForSale": true});
        let priced = json!({"productId": 5, "name": "Gem", "priceInRobux": 10, "isForSale": true});
        let page = |item| json!({"universeId": 77, "requestedPageToken": null, "requestedLimit": 10, "items": [item]});
        let diff = compare_values(
            &env("games products", page(row)),
            &env("games products", page(priced)),
        )
        .unwrap();
        assert!(diff["diff"]["rows"][0]["priceChanged"].is_null());
        assert_eq!(diff["diff"]["unknownPriceIsNotFree"], true);
        let mut after = env(
            "games products",
            json!({"universeId": 77, "requestedPageToken": null, "requestedLimit": 10, "items": []}),
        );
        let before = after.clone();
        after["data"]["result"]
            .as_object_mut()
            .unwrap()
            .remove("requestedPageToken");
        assert_eq!(
            compare_values(&before, &after).unwrap_err().code,
            "snapshot_incompatible"
        );
    }

    #[test]
    fn incompatible_metadata_and_secrets_are_not_echoed() {
        let mut broken = env("charts games", json!({"items": []}));
        broken.as_object_mut().unwrap().remove("cliVersion");
        let err = compare_values(&broken, &broken).unwrap_err();
        assert_eq!(err.code, "snapshot_metadata_missing");
        assert!(!err.message.contains("synthetic-do-not-echo"));
        let raw = br#"{"secret":"synthetic-do-not-echo"}"#;
        let err = parse_bytes(raw)
            .ok()
            .map(|v| compare_values(&v, &v).unwrap_err())
            .unwrap();
        assert_eq!(err.code, "snapshot_not_success");
        assert!(!format!("{err:?}").contains("synthetic-do-not-echo"));
        assert!(parse_bytes(br#"{not-json synthetic-do-not-echo"#).is_err());
    }

    #[test]
    fn report_escapes_formula_and_markdown_and_refuses_overwrite() {
        let input = json!({
            "ok": true, "command": "games products", "cliVersion": "0.7.0",
            "data": {"observedAtUnixMs": 10, "result": {"universeId": 77, "requestedPageToken": "![x](https://tracker.example/a.png)", "requestedLimit": 10, "items": [
                {"productId": 5, "name": "=cmd| <script>", "priceInRobux": null, "isForSale": false}
            ]}}
        });
        let csv = render_value(&input, &RenderFormat::Csv).unwrap();
        assert!(csv.contains("\"'=cmd| <script>\""));
        assert!(csv.contains("unknown"));
        assert!(!csv.contains(",0 Robux") && !csv.contains(",free"));
        let md = render_value(&input, &RenderFormat::Markdown).unwrap();
        assert!(md.contains("\\|"));
        assert!(md.contains("&lt;script&gt;"));
        assert!(!md.contains("<script>"));
        assert!(md.contains("\\!\\[x\\]"));
        assert!(!md.contains("![x](https://tracker.example"));
        assert!(md.contains("Untrusted"));
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("in.json");
        let out = root.path().join("out.csv");
        std::fs::write(&src, serde_json::to_vec(&input).unwrap()).unwrap();
        std::fs::write(&out, b"keep").unwrap();
        let before = std::fs::read(&src).unwrap();
        assert_eq!(
            render_file(&src, &out, RenderFormat::Csv).unwrap_err().code,
            "report_output_exists"
        );
        assert_eq!(std::fs::read(&src).unwrap(), before);
        assert_eq!(std::fs::read(&out).unwrap(), b"keep");
    }
}
