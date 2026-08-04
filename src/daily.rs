use crate::model::{ModelUsage, Usage};
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use std::collections::{BTreeMap, HashMap};

/// 日付・モデル別 usage を加算する。
pub fn add_model_usage(
    daily: &mut BTreeMap<NaiveDate, HashMap<String, Usage>>,
    date: NaiveDate,
    model: &str,
    usage: &Usage,
) {
    daily.entry(date).or_default().entry(model.to_string()).or_default().add(usage);
}

pub fn map_to_daily_models(
    daily: BTreeMap<NaiveDate, HashMap<String, Usage>>,
) -> BTreeMap<NaiveDate, Vec<ModelUsage>> {
    daily
        .into_iter()
        .map(|(date, models)| {
            let models = models
                .into_iter()
                .map(|(model, usage)| ModelUsage { model, usage })
                .collect();
            (date, models)
        })
        .collect()
}

/// 累積 usage の差分（Codex の token_count 用）。
pub fn usage_delta(prev: &Usage, curr: &Usage) -> Usage {
    Usage {
        input_tokens: curr.input_tokens.saturating_sub(prev.input_tokens),
        output_tokens: curr.output_tokens.saturating_sub(prev.output_tokens),
        cache_creation_tokens: curr
            .cache_creation_tokens
            .saturating_sub(prev.cache_creation_tokens),
        cache_read_tokens: curr.cache_read_tokens.saturating_sub(prev.cache_read_tokens),
        estimated: curr.estimated || prev.estimated,
    }
}

/// セッション期間で usage を日割りする（Cursor 等、メッセージ時刻が無い場合）。
pub fn split_usage_by_duration(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    models: &[ModelUsage],
) -> BTreeMap<NaiveDate, Vec<ModelUsage>> {
    let start = start.min(end);
    let end = end.max(start);
    let total_secs = (end - start).num_seconds().max(1) as f64;

    let mut day_ratios: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    let mut date = start.date_naive();
    let end_date = end.date_naive();

    while date <= end_date {
        let day_start = Utc
            .from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
            .max(start);
        let next_day = date + Duration::days(1);
        let day_end = Utc
            .from_utc_datetime(&next_day.and_hms_opt(0, 0, 0).unwrap())
            .min(end);
        let secs = (day_end - day_start).num_seconds() as f64;
        if secs > 0.0 {
            day_ratios.insert(date, secs / total_secs);
        }
        date = next_day;
    }

    let mut out: BTreeMap<NaiveDate, Vec<ModelUsage>> = BTreeMap::new();
    for (day, ratio) in day_ratios {
        if ratio <= 0.0 {
            continue;
        }
        let scaled: Vec<ModelUsage> = models
            .iter()
            .map(|mu| ModelUsage {
                model: mu.model.clone(),
                usage: scale_usage(&mu.usage, ratio),
            })
            .collect();
        out.insert(day, scaled);
    }
    out
}

fn scale_usage(u: &Usage, ratio: f64) -> Usage {
    Usage {
        input_tokens: (u.input_tokens as f64 * ratio).round() as u64,
        output_tokens: (u.output_tokens as f64 * ratio).round() as u64,
        cache_creation_tokens: (u.cache_creation_tokens as f64 * ratio).round() as u64,
        cache_read_tokens: (u.cache_read_tokens as f64 * ratio).round() as u64,
        estimated: u.estimated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn usage_delta_subtracts_cumulative_totals() {
        let prev = Usage {
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        };
        let curr = Usage {
            input_tokens: 30,
            output_tokens: 15,
            ..Default::default()
        };
        let delta = usage_delta(&prev, &curr);
        assert_eq!(delta.input_tokens, 20);
        assert_eq!(delta.output_tokens, 10);
    }

    #[test]
    fn split_usage_by_duration_allocates_across_days() {
        let start = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 7, 2, 12, 0, 0).unwrap();
        let models = vec![ModelUsage {
            model: "gpt-5".into(),
            usage: Usage {
                input_tokens: 100,
                output_tokens: 50,
                ..Default::default()
            },
        }];
        let daily = split_usage_by_duration(start, end, &models);
        assert_eq!(daily.len(), 2);
        let d1 = daily.get(&NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()).unwrap();
        let d2 = daily.get(&NaiveDate::from_ymd_opt(2026, 7, 2).unwrap()).unwrap();
        assert_eq!(d1[0].usage.input_tokens, 50);
        assert_eq!(d2[0].usage.input_tokens, 50);
    }
}
