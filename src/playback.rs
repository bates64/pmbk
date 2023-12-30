use rodio::{OutputStream, source::Source};

use crate::vadpcm::VadpcmDecoder;

pub fn play(decoder: VadpcmDecoder) {
    let duration: Option<std::time::Duration> = decoder.total_duration();

    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    stream_handle.play_raw(decoder).unwrap();
    std::thread::sleep(match duration {
        Some(duration) => duration,
        None => std::time::Duration::from_secs(2),
    });
}
