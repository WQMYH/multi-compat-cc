//! Run against the minisign-verify crate already built by Tauri's updater.
//! Inputs are the decoded public-key box, decoded signature box, artifact and version.

use minisign_verify::{PublicKey, Signature};
use std::{env, error::Error, fs, io};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 4 {
        return Err(
            io::Error::other("expected public key, signature, artifact and version").into(),
        );
    }

    let public_key = PublicKey::from_file(&args[0])?;
    let signature = Signature::from_file(&args[1])?;
    let mut artifact = fs::read(&args[2])?;
    if artifact.is_empty() {
        return Err(io::Error::other("cannot verify an empty artifact").into());
    }
    public_key.verify(&artifact, &signature, false)?;
    let expected_version = args[3]
        .to_str()
        .ok_or_else(|| io::Error::other("version is not valid UTF-8"))?;
    let versions: Vec<_> = signature
        .trusted_comment()
        .split('\t')
        .filter_map(|field| field.strip_prefix("version:"))
        .collect();
    if versions.as_slice() != [expected_version] {
        return Err(io::Error::other("signed version does not match the release").into());
    }

    artifact[0] ^= 1;
    if public_key.verify(&artifact, &signature, false).is_ok() {
        return Err(io::Error::other("tampered artifact was accepted").into());
    }
    artifact[0] ^= 1;

    // An unrelated public test key exercises rejection of the wrong signer.
    let wrong_key =
        PublicKey::from_base64("RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3")?;
    if wrong_key.verify(&artifact, &signature, false).is_ok() {
        return Err(io::Error::other("an unrelated signing key was accepted").into());
    }
    println!("Verified updater signature; tampering and wrong key were rejected.");
    Ok(())
}
