//! Qwen 长录音的连续分段与有界重试；不依赖原生推理库。
use std::ops::{Range, RangeInclusive};

const SAMPLE_RATE: usize = 16_000;
const TARGET_SAMPLES: usize = 20 * SAMPLE_RATE;
const MIN_SEGMENT_SAMPLES: usize = 17 * SAMPLE_RATE;
const MAX_SEGMENT_SAMPLES: usize = 23 * SAMPLE_RATE;
const MIN_RETRY_SAMPLES: usize = 2 * SAMPLE_RATE;
const MAX_SPLIT_DEPTH: usize = 4;
const ENERGY_HALF_WINDOW: usize = SAMPLE_RATE / 100;

pub(crate) struct SegmentTranscript {
    pub text: String,
    pub language: Option<String>,
}

pub(crate) enum SegmentError<E> {
    Retryable(E),
    Fatal(E),
}

#[derive(Debug)]
pub(crate) enum BatchError<E> {
    Revoked,
    Failed { range: Range<usize>, source: E },
}

fn quiet_cut(audio: &[f32], range: RangeInclusive<usize>, preferred: usize) -> usize {
    let mut best = preferred.clamp(*range.start(), *range.end());
    let mut best_energy = f64::INFINITY;
    let mut best_distance = usize::MAX;
    for cut in range.step_by(ENERGY_HALF_WINDOW) {
        let window = &audio[cut.saturating_sub(ENERGY_HALF_WINDOW)
            ..cut.saturating_add(ENERGY_HALF_WINDOW).min(audio.len())];
        let energy = window
            .iter()
            .map(|sample| {
                if sample.is_finite() {
                    f64::from(*sample).powi(2)
                } else {
                    f64::INFINITY
                }
            })
            .sum::<f64>()
            / window.len().max(1) as f64;
        let distance = cut.abs_diff(preferred);
        if energy < best_energy || (energy == best_energy && distance < best_distance) {
            best = cut;
            best_energy = energy;
            best_distance = distance;
        }
    }
    best
}

fn segment_ranges(audio: &[f32]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    while audio.len() - start > MAX_SEGMENT_SAMPLES {
        // 给尾段保留至少两秒，不将刚超出上限的录音切出极短尾巴。
        let last_cut = (start + MAX_SEGMENT_SAMPLES).min(audio.len() - MIN_RETRY_SAMPLES);
        let end = quiet_cut(
            audio,
            start + MIN_SEGMENT_SAMPLES..=last_cut,
            start + TARGET_SAMPLES,
        );
        ranges.push(start..end);
        start = end;
    }
    if start < audio.len() {
        ranges.push(start..audio.len());
    }
    ranges
}

fn is_unspaced_script(character: char) -> bool {
    matches!(character as u32,
        0x2e80..=0x9fff | 0xf900..=0xfaff |
        0xfe30..=0xfe4f | 0xff00..=0xffef | 0x20000..=0x323af)
}

fn append_text(output: &mut String, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    if let (Some(left), Some(right)) = (output.chars().next_back(), text.chars().next()) {
        // 中文按原文相接；空格分词语言避免把相邻段的首尾词粘在一起。
        let closes_punctuation =
            matches!(right, '.' | ',' | '!' | '?' | ';' | ':' | '%' | ')' | ']');
        let opens_punctuation = matches!(left, '(' | '[' | '“' | '‘');
        if !is_unspaced_script(left)
            && !is_unspaced_script(right)
            && !closes_punctuation
            && !opens_punctuation
        {
            output.push(' ');
        }
    }
    output.push_str(text);
}

/// 仅成功段可进入结果。截断片段必须整体重做，任何终态失败都不返回半份转写。
pub(crate) fn transcribe<E>(
    audio: &[f32],
    mut current: impl FnMut() -> bool,
    mut run: impl FnMut(&[f32]) -> Result<SegmentTranscript, SegmentError<E>>,
) -> Result<SegmentTranscript, BatchError<E>> {
    let mut pending: Vec<_> = segment_ranges(audio)
        .into_iter()
        .rev()
        .map(|range| (range, 0))
        .collect();
    let mut result = SegmentTranscript {
        text: String::new(),
        language: None,
    };
    let mut conflicting_languages = false;
    let mut missing_language = false;
    while let Some((range, depth)) = pending.pop() {
        if !current() {
            return Err(BatchError::Revoked);
        }
        let attempt = run(&audio[range.clone()]);
        if !current() {
            return Err(BatchError::Revoked);
        }
        match attempt {
            Ok(transcript) => {
                append_text(&mut result.text, &transcript.text);
                let language = transcript.language.filter(|value| !value.trim().is_empty());
                if let Some(language) = language {
                    if result
                        .language
                        .as_ref()
                        .is_some_and(|value| *value != language)
                    {
                        conflicting_languages = true;
                    }
                    if !conflicting_languages {
                        result.language = Some(language);
                    }
                } else if !transcript.text.trim().is_empty() {
                    missing_language = true;
                }
            }
            Err(SegmentError::Retryable(source))
                if depth < MAX_SPLIT_DEPTH && range.len() >= 2 * MIN_RETRY_SAMPLES =>
            {
                // 有界二分时也优先在中点附近低能量处切分；两侧均不小于两秒。
                let middle = range.start + range.len() / 2;
                let cut = quiet_cut(
                    audio,
                    (middle.saturating_sub(SAMPLE_RATE)).max(range.start + MIN_RETRY_SAMPLES)
                        ..=(middle + SAMPLE_RATE).min(range.end - MIN_RETRY_SAMPLES),
                    middle,
                );
                drop(source);
                pending.push((cut..range.end, depth + 1));
                pending.push((range.start..cut, depth + 1));
            }
            Err(SegmentError::Retryable(source) | SegmentError::Fatal(source)) => {
                return Err(BatchError::Failed { range, source });
            }
        }
    }
    if conflicting_languages || missing_language {
        result.language = None;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn success(
        text: &str,
        language: Option<&str>,
    ) -> Result<SegmentTranscript, SegmentError<&'static str>> {
        Ok(SegmentTranscript {
            text: text.to_owned(),
            language: language.map(str::to_owned),
        })
    }

    #[test]
    fn segments_cover_every_sample_once_in_order_including_tail() {
        for length in [
            0,
            1,
            2 * SAMPLE_RATE,
            MAX_SEGMENT_SAMPLES,
            MAX_SEGMENT_SAMPLES + 1,
            40 * SAMPLE_RATE + 7,
            159 * SAMPLE_RATE + 3_840,
        ] {
            let audio = vec![0.0; length];
            let ranges = segment_ranges(&audio);
            let mut next = 0;
            for range in ranges {
                assert_eq!(range.start, next);
                assert!(!range.is_empty());
                assert!(range.len() <= MAX_SEGMENT_SAMPLES);
                next = range.end;
            }
            assert_eq!(next, length);
        }
    }

    #[test]
    fn silence_prefers_twenty_seconds_and_retains_short_last_audio() {
        let audio = vec![0.0; 41 * SAMPLE_RATE];
        assert_eq!(
            segment_ranges(&audio),
            [0..20 * SAMPLE_RATE, 20 * SAMPLE_RATE..41 * SAMPLE_RATE]
        );
        let mut called = 0;
        let result = transcribe(
            &[],
            || true,
            |_| {
                called += 1;
                success("unexpected", None)
            },
        )
        .unwrap();
        assert_eq!(called, 0);
        assert!(result.text.is_empty());
    }

    #[test]
    fn cuts_in_low_energy_window_and_avoids_tiny_tail() {
        let mut audio = vec![0.5; 45 * SAMPLE_RATE];
        audio[18 * SAMPLE_RATE..19 * SAMPLE_RATE].fill(0.0);
        let ranges = segment_ranges(&audio);
        assert!((18 * SAMPLE_RATE..=19 * SAMPLE_RATE).contains(&ranges[0].end));
        let near_limit = segment_ranges(&vec![0.0; MAX_SEGMENT_SAMPLES + 1]);
        assert!(near_limit.last().unwrap().len() >= MIN_RETRY_SAMPLES);
    }

    #[test]
    fn invalid_samples_do_not_break_coverage_or_cut_search() {
        let audio = vec![f32::NAN; 70 * SAMPLE_RATE];
        let ranges = segment_ranges(&audio);
        assert_eq!(ranges.first().unwrap().start, 0);
        assert_eq!(ranges.last().unwrap().end, audio.len());
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
    }

    #[test]
    fn joins_chinese_english_and_punctuation_without_partial_deduplication() {
        for (pieces, expected) in [
            (vec![" 第一段。 ", "第二段", "。"], "第一段。第二段。"),
            (vec![" Hello, ", "world", "."], "Hello, world."),
            (vec!["重复", "", "重复"], "重复重复"),
            (vec!["使用", "Inputia", "输入。"], "使用Inputia输入。"),
            (vec!["안녕하세요", "여러분"], "안녕하세요 여러분"),
        ] {
            let mut joined = String::new();
            for piece in pieces {
                append_text(&mut joined, piece);
            }
            assert_eq!(joined, expected);
        }
    }

    #[test]
    fn retry_subdivides_only_failed_audio_without_gaps_or_overlap() {
        let audio: Vec<f32> = (0..20 * SAMPLE_RATE).map(|index| index as f32).collect();
        let mut successful_ranges = Vec::new();
        let mut attempts = 0;
        let result = transcribe(
            &audio,
            || true,
            |samples| {
                attempts += 1;
                if samples.len() > 6 * SAMPLE_RATE {
                    return Err(SegmentError::Retryable(
                        "truncated partial must be discarded",
                    ));
                }
                successful_ranges
                    .push(samples[0] as usize..samples.last().unwrap().to_owned() as usize + 1);
                success("完成", Some("zh"))
            },
        )
        .unwrap();
        assert!(attempts > successful_ranges.len());
        let mut next = 0;
        for range in successful_ranges.iter() {
            assert_eq!(range.start, next);
            next = range.end;
        }
        assert_eq!(next, audio.len());
        assert_eq!(result.text, "完成".repeat(successful_ranges.len()));
        assert_eq!(result.language.as_deref(), Some("zh"));
    }

    #[test]
    fn persistent_truncation_stops_at_bounded_minimum() {
        let audio = vec![0.0; 23 * SAMPLE_RATE];
        let mut attempts = 0;
        let error = transcribe(
            &audio,
            || true,
            |_| {
                attempts += 1;
                Err(SegmentError::Retryable("truncated"))
            },
        )
        .err()
        .unwrap();
        assert!(attempts <= MAX_SPLIT_DEPTH + 1);
        match error {
            BatchError::Failed { range, source } => {
                assert_eq!(source, "truncated");
                assert!(range.len() >= MIN_RETRY_SAMPLES);
                assert!(range.len() < 2 * MIN_RETRY_SAMPLES);
            }
            BatchError::Revoked => panic!("unexpected revocation"),
        }
    }

    #[test]
    fn non_retryable_error_stops_immediately_without_partial_success() {
        let audio = vec![0.0; 60 * SAMPLE_RATE];
        let mut attempts = 0;
        let error = transcribe(
            &audio,
            || true,
            |_| {
                attempts += 1;
                if attempts == 1 {
                    success("不能返回这份部分正文", Some("zh"))
                } else {
                    Err(SegmentError::Fatal("backend failure"))
                }
            },
        )
        .err()
        .unwrap();
        assert_eq!(attempts, 2);
        assert!(matches!(
            error,
            BatchError::Failed {
                source: "backend failure",
                ..
            }
        ));
    }

    #[test]
    fn revoked_before_dispatch_runs_nothing() {
        let mut attempts = 0;
        let error = transcribe(
            &vec![0.0; 60 * SAMPLE_RATE],
            || false,
            |_| {
                attempts += 1;
                success("unexpected", None)
            },
        )
        .err()
        .unwrap();
        assert_eq!(attempts, 0);
        assert!(matches!(error, BatchError::Revoked));
    }

    #[test]
    fn revocation_after_run_discards_result_and_prevents_next_segment_or_retry() {
        for retryable in [false, true] {
            let valid = Cell::new(true);
            let mut attempts = 0;
            let error = transcribe(
                &vec![0.0; 60 * SAMPLE_RATE],
                || valid.get(),
                |_| {
                    attempts += 1;
                    valid.set(false);
                    if retryable {
                        Err(SegmentError::Retryable("truncated"))
                    } else {
                        success("discarded", None)
                    }
                },
            )
            .err()
            .unwrap();
            assert_eq!(attempts, 1);
            assert!(matches!(error, BatchError::Revoked));
        }
    }

    #[test]
    fn mixed_segment_languages_do_not_replace_evidence_with_last_language() {
        for languages in [
            [Some("en"), Some("zh"), Some("zh")],
            [Some("en"), None, Some("en")],
        ] {
            let mut languages = languages.into_iter();
            let result = transcribe(
                &vec![0.0; 60 * SAMPLE_RATE],
                || true,
                |_| success("text", languages.next().flatten()),
            )
            .unwrap();
            assert_eq!(result.text, "text text text");
            assert_eq!(result.language, None);
        }
    }
}
