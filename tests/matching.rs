use std::error::Error;
use std::fs::{read_dir, File};
use std::io::{prelude::*, Cursor};

use pmbk::Bank;

// Copy/link papermario/assets/*/audio directory to tests/audio
const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/audio");

#[test]
fn matching() -> Result<(), Box<dyn Error>> {
    // binread then binwrite every bk file

    for entry in read_dir(DIR)? {
        let path = entry?.path();
        if path.extension().map(|e| e == "bk").unwrap_or(false) {
            let mut original = Vec::new();
            File::open(path)?.read_to_end(&mut original)?;

            let bank = Bank::read(&mut Cursor::new(&mut original))?;

            println!("bank: {}", bank.name());

            let mut rewritten = Vec::new();
            bank.write(&mut Cursor::new(&mut rewritten))?;

            // write to tmp
            let mut tmp = File::create("bad.bk")?;
            tmp.write_all(&rewritten)?;

            assert_eq!(original.len(), rewritten.len());
            assert_eq!(original, rewritten);
        }
    }

    Ok(())
}
