use super::super::audio::TunerReadout;
use super::super::plugin_runtime::ParameterSnapshot;

pub(crate) fn category_segments(category: &str) -> Vec<String> {
    category
        .split('|')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(|segment| match segment {
            "Fx" => "Effects".to_owned(),
            other => other.to_owned(),
        })
        .collect()
}

pub(crate) fn effect_type(category: &str) -> String {
    category_segments(category)
        .into_iter()
        .find(|segment| !segment.eq_ignore_ascii_case("Effects"))
        .unwrap_or_else(|| "Other".to_owned())
}

pub(crate) fn format_time(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

pub(crate) fn tuner_note(
    readout: TunerReadout,
    a4_pitch_hz: f32,
    transposition_semitones: i32,
) -> Option<String> {
    if readout.confidence < 0.25 || readout.frequency_hz <= 0.0 {
        return None;
    }
    const NOTE_NAMES: [&str; 12] = [
        "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
    ];
    let midi = (69.0
        + 12.0 * (readout.frequency_hz / a4_pitch_hz).log2()
        + transposition_semitones as f32)
        .round() as i32;
    let note_name = NOTE_NAMES[midi.rem_euclid(12) as usize];
    let octave = midi.div_euclid(12) - 1;
    Some(format!("{note_name}{octave}"))
}

pub(crate) fn format_cents(cents: f32) -> String {
    if cents.abs() < 0.05 {
        "in tune".to_owned()
    } else {
        format!("{cents:+.1} cents")
    }
}

pub(crate) fn move_item<T>(items: &mut Vec<T>, from: usize, to: usize) {
    if from >= items.len() || to > items.len() {
        return;
    }
    let item = items.remove(from);
    let adjusted_to = if from < to { to - 1 } else { to };
    items.insert(adjusted_to, item);
}

pub(crate) fn is_booleanish_parameter(
    parameter: &ParameterSnapshot,
    formatted_value: &str,
) -> bool {
    parameter.step_count == 1
        || boolean_display_value(formatted_value).is_some()
        || parameter
            .name
            .split(|character: char| !character.is_alphanumeric())
            .any(|word| {
                matches!(
                    word.to_ascii_lowercase().as_str(),
                    "active"
                        | "bypass"
                        | "disable"
                        | "disabled"
                        | "enable"
                        | "enabled"
                        | "mute"
                        | "on"
                        | "off"
                        | "solo"
                )
            })
}

pub(crate) fn boolean_parameter_value(
    parameter: &ParameterSnapshot,
    formatted_value: &str,
) -> bool {
    boolean_display_value(formatted_value).unwrap_or_else(|| {
        let midpoint = (parameter.min + parameter.max) / 2.0;
        parameter.value >= midpoint
    })
}

pub(crate) fn format_custom_editor_float(value: f64) -> String {
    format!("{value:.2}")
}

fn boolean_display_value(formatted_value: &str) -> Option<bool> {
    match formatted_value.trim().to_ascii_lowercase().as_str() {
        "active" | "enabled" | "on" | "true" | "yes" => Some(true),
        "disabled" | "inactive" | "no" | "off" | "false" => Some(false),
        _ => None,
    }
}

pub(crate) fn group_parameter_indices(
    parameters: &[ParameterSnapshot],
) -> Vec<(String, Vec<usize>)> {
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();

    for (index, parameter) in parameters.iter().enumerate() {
        let category = parameter.category.trim();
        let category = if category.is_empty() || category.eq_ignore_ascii_case("general") {
            "General".to_owned()
        } else {
            category.to_owned()
        };

        if let Some((_, indices)) = groups
            .iter_mut()
            .find(|(existing_category, _)| existing_category == &category)
        {
            indices.push(index);
        } else {
            groups.push((category, vec![index]));
        }
    }

    groups.sort_by_key(|(category, _)| !category.eq_ignore_ascii_case("General"));
    groups
}

pub(crate) fn balanced_column_ranges(
    item_count: usize,
    column_count: usize,
) -> Vec<(usize, usize)> {
    if column_count == 0 {
        return Vec::new();
    }

    let items_per_column = item_count / column_count;
    let remainder = item_count % column_count;
    let mut start = 0;

    (0..column_count)
        .map(|column| {
            let end = start + items_per_column + usize::from(column < remainder);
            let range = (start, end);
            start = end;
            range
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        balanced_column_ranges, effect_type, format_custom_editor_float, group_parameter_indices,
        move_item,
    };
    use crate::app::plugin_runtime::ParameterSnapshot;

    fn parameter(category: &str) -> ParameterSnapshot {
        ParameterSnapshot {
            id: 0,
            name: String::new(),
            category: category.to_owned(),
            min: 0.0,
            max: 1.0,
            value: 0.0,
            step_count: 0,
            is_read_only: false,
            formatted_value: String::new(),
        }
    }

    #[test]
    fn rounds_custom_editor_float_values_to_two_decimal_places() {
        assert_eq!(format_custom_editor_float(1.236), "1.24");
        assert_eq!(format_custom_editor_float(1.2), "1.20");
    }

    #[test]
    fn extracts_effect_type_from_plugin_category() {
        assert_eq!(effect_type("Fx|Delay"), "Delay");
        assert_eq!(effect_type("Fx|Dynamics|Compressor"), "Dynamics");
        assert_eq!(effect_type("Fx"), "Other");
    }

    #[test]
    fn balances_parameters_across_three_columns() {
        assert_eq!(balanced_column_ranges(7, 3), [(0, 3), (3, 5), (5, 7)]);
        assert_eq!(balanced_column_ranges(2, 3), [(0, 1), (1, 2), (2, 2)]);
        assert_eq!(balanced_column_ranges(4, 0), []);
    }

    #[test]
    fn puts_general_parameters_in_the_first_category() {
        let groups = group_parameter_indices(&[
            parameter("Modulation"),
            parameter(""),
            parameter("general"),
            parameter("Dynamics"),
        ]);

        assert_eq!(groups[0], ("General".to_owned(), vec![1, 2]));
        assert_eq!(groups[1], ("Modulation".to_owned(), vec![0]));
        assert_eq!(groups[2], ("Dynamics".to_owned(), vec![3]));
    }

    #[test]
    fn moves_an_item_before_a_drop_index() {
        let mut items = vec!["input", "compressor", "delay", "output"];

        move_item(&mut items, 2, 1);

        assert_eq!(items, ["input", "delay", "compressor", "output"]);
    }

    #[test]
    fn moves_an_item_to_the_end_of_the_stack() {
        let mut items = vec!["input", "compressor", "delay", "output"];
        let end = items.len();

        move_item(&mut items, 0, end);

        assert_eq!(items, ["compressor", "delay", "output", "input"]);
    }

    #[test]
    fn dropping_next_to_the_same_item_is_a_noop() {
        let mut items = vec!["input", "compressor", "delay"];

        move_item(&mut items, 1, 2);

        assert_eq!(items, ["input", "compressor", "delay"]);
    }
}
