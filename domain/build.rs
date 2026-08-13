fn main() {
    utoipa_config::Config::new()
        .alias_for("Id", "Uuid")
        .alias_for("DateTimeWithTimeZone", "DateTime<FixedOffset>")
        .write_to_file()
}
