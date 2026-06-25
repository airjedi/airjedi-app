use shadow_rs::shadow;

shadow!(build);

/// Short version string for the status bar: "v0.1.0-af6869e (2026-03-06 15:08)"
pub fn version_short() -> String {
    let hash = &build::SHORT_COMMIT[..7.min(build::SHORT_COMMIT.len())];
    let dirty = if build::GIT_CLEAN { "" } else { "*" };
    // Trim seconds and timezone from BUILD_TIME ("2026-03-06 15:08:41 -06:00" -> "2026-03-06 15:08")
    let build_time = build::BUILD_TIME.get(..16).unwrap_or(build::BUILD_TIME);
    format!("v{}-{}{} ({})", build::PKG_VERSION, hash, dirty, build_time)
}
