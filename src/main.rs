use binrw::BinRead;
use pmbk::*;
use std::error::Error;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about)]
/// BK file decoder. Writes decoded instruments to data/NAME_INDEX.wav
struct Args {
    /// BK file to read
    input: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    let file = std::fs::File::open(&args.input)?;
    let mut bufreader = std::io::BufReader::new(file);
    let bk: Bk = Bk::read(&mut bufreader)?;

    for (i, instrument) in bk.instruments().iter().enumerate() {
        #[cfg(feature = "wav")]
        {
            let mut file = std::fs::File::create(format!("data/{}_{}.wav", bk.name(), i))?;
            instrument.clone().write_wav(&mut file)?;
        }

        #[cfg(feature = "rodio")]
        playback::play(vadpcm::VadpcmDecoder::new(instrument.clone()));
    }

    Ok(())
}
