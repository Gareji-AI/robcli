use crate::error::AppError;

pub(crate) fn archive_name(version: &str, os: &str, arch: &str) -> Result<String, AppError> {
    if !version.chars().all(|c| c.is_ascii_digit() || c == '.')
        || version.is_empty()
        || version.starts_with('.')
        || version.ends_with('.')
        || version.contains("..")
        || !matches!(
            (os, arch),
            ("windows", "x86_64") | ("linux", "x86_64") | ("macos", "aarch64")
        )
    {
        return Err(AppError::new(
            "archive_name_invalid",
            "The release archive name is outside the approved target matrix.",
            "Use windows/x86_64, linux/x86_64, or macos/aarch64 and a plain numeric version.",
        ));
    }
    let ext = if os == "windows" { "zip" } else { "tar.gz" };
    Ok(format!("robcli-{version}-{os}-{arch}.{ext}"))
}

pub(crate) fn members(os: &str) -> Result<[&'static str; 3], AppError> {
    match os {
        "windows" => Ok(["rob.exe", "license-mit", "license-apache"]),
        "macos" | "linux" => Ok(["rob", "license-mit", "license-apache"]),
        _ => Err(AppError::new(
            "archive_name_invalid",
            "The release archive name is outside the approved target matrix.",
            "Use windows/x86_64, linux/x86_64, or macos/aarch64 and a plain numeric version.",
        )),
    }
}

pub(crate) fn accept_members(os: &str, names: &[&str]) -> Result<(), AppError> {
    let allowed = members(os)?;
    if names.len() != allowed.len() || names.iter().any(|name| !allowed.contains(name)) {
        return Err(AppError::new(
            "archive_members_rejected",
            "The release archive contains a file outside the CLI and license allowlist.",
            "Include only the rob binary and the two license files. Do not add profiles, cookies, keys, traffic state, or build artifacts.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_members_stay_inside_the_approved_matrix() {
        assert_eq!(
            archive_name("0.7.0", "windows", "x86_64").unwrap(),
            "robcli-0.7.0-windows-x86_64.zip"
        );
        assert_eq!(
            archive_name("0.7.0", "macos", "aarch64").unwrap(),
            "robcli-0.7.0-macos-aarch64.tar.gz"
        );
        assert!(archive_name("0.7.0", "macos", "x86_64").is_err());
        assert!(archive_name("0.7.0-dev", "linux", "x86_64").is_err());
        accept_members("linux", &["rob", "license-mit", "license-apache"]).unwrap();
        assert!(
            accept_members(
                "linux",
                &["rob", "license-mit", "license-apache", "traffic.json"]
            )
            .is_err()
        );
        assert!(accept_members("windows", &["rob", "license-mit", "license-apache"]).is_err());
    }
}
