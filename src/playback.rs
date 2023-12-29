use rodio::{OutputStream, source::Source};
use rodio::buffer::SamplesBuffer;

use crate::Instrument;

pub fn play(instrument: &Instrument, pcm: &[i16]) {
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let source = SamplesBuffer::new(1, instrument.output_rate as u32, pcm);
    stream_handle.play_raw(source.convert_samples()).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(1));
}
