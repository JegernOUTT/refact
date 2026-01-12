use std::io::Cursor;
use symphonia::core::audio::AudioBufferRef;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

const TARGET_SAMPLE_RATE: u32 = 16000;

pub fn decode_audio(data: &[u8], mime_type: &str) -> Result<Vec<f32>, String> {
    let cursor = Cursor::new(data.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let mut hint = Hint::new();
    match mime_type {
        "audio/wav" | "audio/wave" | "audio/x-wav" => hint.with_extension("wav"),
        "audio/webm" => hint.with_extension("webm"),
        "audio/ogg" => hint.with_extension("ogg"),
        "audio/mpeg" | "audio/mp3" => hint.with_extension("mp3"),
        _ => &mut hint,
    };

    let format_opts = FormatOptions::default();
    let metadata_opts = MetadataOptions::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &format_opts, &metadata_opts)
        .map_err(|e| format!("Failed to probe audio format: {:?}", e))?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or("No supported audio track found")?;

    let decoder_opts = DecoderOptions::default();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &decoder_opts)
        .map_err(|e| format!("Failed to create decoder: {:?}", e))?;

    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(1);

    let mut samples: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(format!("Failed to read packet: {:?}", e)),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = decoder
            .decode(&packet)
            .map_err(|e| format!("Failed to decode packet: {:?}", e))?;

        append_samples(&decoded, &mut samples);
    }

    let mono = to_mono(&samples, channels);
    let resampled = resample(&mono, sample_rate, TARGET_SAMPLE_RATE)?;

    Ok(resampled)
}

fn append_samples(buffer: &AudioBufferRef, output: &mut Vec<f32>) {
    match buffer {
        AudioBufferRef::F32(buf) => {
            for plane in buf.planes().planes() {
                output.extend_from_slice(plane);
            }
        }
        AudioBufferRef::S16(buf) => {
            for plane in buf.planes().planes() {
                output.extend(plane.iter().map(|&s| s as f32 / 32768.0));
            }
        }
        AudioBufferRef::S32(buf) => {
            for plane in buf.planes().planes() {
                output.extend(plane.iter().map(|&s| s as f32 / 2147483648.0));
            }
        }
        AudioBufferRef::U8(buf) => {
            for plane in buf.planes().planes() {
                output.extend(plane.iter().map(|&s| (s as f32 - 128.0) / 128.0));
            }
        }
        _ => {}
    }
}

fn to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels == 1 {
        return samples.to_vec();
    }

    samples
        .chunks(channels)
        .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
        .collect()
}

fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Result<Vec<f32>, String> {
    if from_rate == to_rate {
        return Ok(samples.to_vec());
    }

    use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};

    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };

    let mut resampler = SincFixedIn::<f32>::new(
        to_rate as f64 / from_rate as f64,
        2.0,
        params,
        samples.len(),
        1,
    ).map_err(|e| format!("Failed to create resampler: {:?}", e))?;

    let input = vec![samples.to_vec()];
    let output = resampler
        .process(&input, None)
        .map_err(|e| format!("Resampling failed: {:?}", e))?;

    Ok(output.into_iter().next().unwrap_or_default())
}
