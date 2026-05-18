use gpui::SharedString;
use std::time::SystemTime;

pub(crate) fn format_byte_size(size: Option<u64>) -> SharedString {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

    let Some(size) = size else {
        return "--".into();
    };

    let mut value = size as f64;
    let mut unit_index = 0;
    while value >= 1024.0 && unit_index < UNITS.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{size} {}", UNITS[unit_index]).into()
    } else {
        format!("{value:.1} {}", UNITS[unit_index]).into()
    }
}

pub(crate) fn format_local_timestamp(value: Option<SystemTime>) -> SharedString {
    value
        .map(|value| {
            let datetime = time::OffsetDateTime::from(value);
            let datetime = time::UtcOffset::current_local_offset()
                .ok()
                .map(|offset| datetime.to_offset(offset))
                .unwrap_or(datetime);

            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                datetime.year(),
                datetime.month() as u8,
                datetime.day(),
                datetime.hour(),
                datetime.minute(),
                datetime.second(),
            )
        })
        .map(Into::into)
        .unwrap_or_else(|| "--".into())
}
