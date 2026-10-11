use super::types::ChannelRouting;

/// Maps mono device channels to the stereo processing layout and adapts stereo
/// output for mono devices.
pub(crate) fn normalize_routing(
    routing: ChannelRouting,
    input_channels: usize,
    output_channels: usize,
) -> ChannelRouting {
    ChannelRouting {
        input_left: if input_channels == 1 {
            0
        } else {
            routing.input_left
        },
        input_right: if input_channels == 1 {
            0
        } else {
            routing.input_right
        },
        output_left: if output_channels == 1 {
            0
        } else {
            routing.output_left
        },
        output_right: if output_channels == 1 {
            0
        } else {
            routing.output_right
        },
    }
}

pub(crate) fn route_input_sample(
    channel: usize,
    input_channels: usize,
    sample: f32,
    input_left: usize,
    input_right: usize,
    left: &mut f32,
    right: &mut f32,
) {
    if input_channels == 1 {
        *left = sample;
        *right = sample;
    } else {
        if channel == input_left {
            *left = sample;
        }
        if channel == input_right {
            *right = sample;
        }
    }
}

pub(crate) fn route_output_sample(
    frame: &mut [f32],
    output_left: usize,
    output_right: usize,
    left: f32,
    right: f32,
) {
    if frame.len() == 1 {
        // Preserve both stereo sides when adapting to mono, with unity gain for
        // a dual-mono signal.
        frame[0] = left * 0.5 + right * 0.5;
        return;
    }

    if let Some(output) = frame.get_mut(output_left) {
        *output = left;
    }
    if let Some(output) = frame.get_mut(output_right) {
        *output = right;
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_routing, route_input_sample, route_output_sample};
    use crate::app::audio::types::ChannelRouting;

    #[test]
    fn mono_input_is_mirrored_into_both_stereo_channels() {
        let mut left = 0.0;
        let mut right = 0.0;

        route_input_sample(0, 1, 0.75, 0, 0, &mut left, &mut right);

        assert_eq!([left, right], [0.75, 0.75]);
    }

    #[test]
    fn stereo_input_keeps_independent_channel_routing() {
        let mut left = 0.0;
        let mut right = 0.0;

        route_input_sample(0, 2, 0.25, 1, 0, &mut left, &mut right);
        route_input_sample(1, 2, -0.5, 1, 0, &mut left, &mut right);

        assert_eq!([left, right], [-0.5, 0.25]);
    }

    #[test]
    fn stereo_output_is_averaged_for_mono_device() {
        let mut frame = [0.0];

        route_output_sample(&mut frame, 0, 0, 0.75, 0.25);

        assert_eq!(frame, [0.5]);
    }

    #[test]
    fn stereo_output_keeps_independent_channel_routing() {
        let mut frame = [0.0; 2];

        route_output_sample(&mut frame, 1, 0, 0.75, -0.5);

        assert_eq!(frame, [-0.5, 0.75]);
    }

    #[test]
    fn mono_device_routing_ignores_unavailable_stereo_channel_indices() {
        let routing = normalize_routing(
            ChannelRouting {
                input_left: 0,
                input_right: 1,
                output_left: 0,
                output_right: 1,
            },
            1,
            1,
        );

        assert_eq!(
            routing,
            ChannelRouting {
                input_left: 0,
                input_right: 0,
                output_left: 0,
                output_right: 0,
            }
        );
    }
}
