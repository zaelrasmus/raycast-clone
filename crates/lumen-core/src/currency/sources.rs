//! Where tables come from.
//!
//! Three services, one shape: a GET that answers every rate against USD.
//! ExchangeRate-API's open and keyed endpoints return the same document
//! (`rates` in one, `conversion_rates` in the other) with the moments it was
//! published and is next due; UniRateAPI returns only the rates, so its table
//! is taken as published now and due in a day.
//!
//! A URL is never logged: the keyed ones carry the key.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::{CurrencyConfig, RateSource, Rates, Service, When};

/// How long a fetch may take before it is reported as failed.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Every table is fetched against this, and converted across it.
const BASE: &str = "USD";

/// A day, for a service that does not say when its next table is due.
const DAY: i64 = 86_400;

/// The source `config` names, with a client ready to use.
pub fn source(config: &CurrencyConfig) -> Result<Box<dyn RateSource>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("Lumen/", env!("CARGO_PKG_VERSION")))
        .build()?;

    let key = || match config.key() {
        Some(key) => Ok(key.to_string()),
        None => bail!(
            "{} needs an API key: set currency.api-key in config.json",
            config.service.label()
        ),
    };

    Ok(match config.service {
        Service::Open => Box::new(ExchangeRateApi { client, key: None }),
        Service::ExchangerateApi => Box::new(ExchangeRateApi {
            client,
            key: Some(key()?),
        }),
        Service::Unirate => Box::new(UniRate {
            client,
            key: key()?,
        }),
    })
}

/// ExchangeRate-API, keyless (`open.er-api.com`) or keyed.
struct ExchangeRateApi {
    client: reqwest::blocking::Client,
    key: Option<String>,
}

#[derive(Deserialize)]
struct ExchangeRateAnswer {
    result: String,
    #[serde(rename = "error-type")]
    error_type: Option<String>,
    base_code: Option<String>,
    #[serde(alias = "conversion_rates")]
    rates: Option<BTreeMap<String, f64>>,
    time_last_update_unix: Option<i64>,
    time_next_update_unix: Option<i64>,
}

impl RateSource for ExchangeRateApi {
    fn fetch(&self, when: When) -> Result<Rates> {
        let When::Latest = when else {
            bail!("ExchangeRate-API's historical rates need a paid plan");
        };
        let (url, service) = match &self.key {
            None => (
                format!("https://open.er-api.com/v6/latest/{BASE}"),
                Service::Open,
            ),
            Some(key) => (
                format!("https://v6.exchangerate-api.com/v6/{key}/latest/{BASE}"),
                Service::ExchangerateApi,
            ),
        };

        let response = self
            .client
            .get(url)
            .send()
            .map_err(reqwest::Error::without_url)
            .context("reaching ExchangeRate-API")?;
        let status = response.status();
        let body = response
            .text()
            .map_err(reqwest::Error::without_url)
            .context("reading ExchangeRate-API's answer")?;
        let answer: ExchangeRateAnswer = serde_json::from_str(&body)
            .with_context(|| format!("ExchangeRate-API answered {status}"))?;
        parse_exchange_rate(answer, service, status)
    }
}

fn parse_exchange_rate(
    answer: ExchangeRateAnswer,
    service: Service,
    status: reqwest::StatusCode,
) -> Result<Rates> {
    if answer.result != "success" {
        let kind = answer.error_type.unwrap_or_else(|| status.to_string());
        bail!(match kind.as_str() {
            "invalid-key" | "inactive-account" => "ExchangeRate-API refused the API key".into(),
            "quota-reached" => "The ExchangeRate-API quota for this month is spent".into(),
            _ => format!("ExchangeRate-API answered {kind}"),
        });
    }
    let (Some(rates), Some(published), Some(next)) = (
        answer.rates,
        answer.time_last_update_unix,
        answer.time_next_update_unix,
    ) else {
        bail!("ExchangeRate-API answered without rates");
    };
    Ok(Rates {
        service,
        base: answer.base_code.unwrap_or_else(|| BASE.into()),
        rates,
        published,
        next,
    })
}

/// UniRateAPI, keyed.
struct UniRate {
    client: reqwest::blocking::Client,
    key: String,
}

#[derive(Deserialize)]
struct UniRateAnswer {
    base: Option<String>,
    rates: Option<BTreeMap<String, serde_json::Value>>,
    error: Option<String>,
}

impl RateSource for UniRate {
    fn fetch(&self, when: When) -> Result<Rates> {
        let When::Latest = when else {
            bail!("UniRateAPI's historical rates need a paid plan");
        };
        let response = self
            .client
            .get("https://api.unirateapi.com/api/rates")
            .query(&[("from", BASE), ("api_key", self.key.as_str())])
            .send()
            .map_err(reqwest::Error::without_url)
            .context("reaching UniRateAPI")?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            bail!("UniRateAPI refused the API key");
        }
        let body = response
            .text()
            .map_err(reqwest::Error::without_url)
            .context("reading UniRateAPI's answer")?;
        let answer: UniRateAnswer =
            serde_json::from_str(&body).with_context(|| format!("UniRateAPI answered {status}"))?;
        let now = jiff::Timestamp::now().as_second();
        parse_unirate(answer, status, now)
    }
}

fn parse_unirate(answer: UniRateAnswer, status: reqwest::StatusCode, now: i64) -> Result<Rates> {
    let Some(written) = answer.rates.filter(|_| status.is_success()) else {
        let message = answer.error.unwrap_or_else(|| status.to_string());
        bail!("UniRateAPI answered {message}");
    };
    // Numbers, or numbers written as strings.
    let rates = written
        .into_iter()
        .filter_map(|(code, value)| {
            let rate = match value {
                serde_json::Value::Number(number) => number.as_f64(),
                serde_json::Value::String(text) => text.parse().ok(),
                _ => None,
            }?;
            Some((code, rate))
        })
        .collect();
    Ok(Rates {
        service: Service::Unirate,
        base: answer.base.unwrap_or_else(|| BASE.into()),
        rates,
        published: now,
        next: now + DAY,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn exchange_rate_api_reads_both_spellings_of_its_rates() {
        for field in ["rates", "conversion_rates"] {
            let answer: ExchangeRateAnswer = serde_json::from_str(&format!(
                r#"{{ "result": "success", "base_code": "USD",
                     "time_last_update_unix": 10, "time_next_update_unix": 20,
                     "{field}": {{ "USD": 1, "CAD": 1.41 }} }}"#
            ))
            .unwrap();
            let rates = parse_exchange_rate(answer, Service::Open, StatusCode::OK).unwrap();
            assert_eq!(rates.rates["CAD"], 1.41);
            assert_eq!((rates.published, rates.next), (10, 20));
        }
    }

    #[test]
    fn a_refused_key_is_said_as_such() {
        let answer: ExchangeRateAnswer =
            serde_json::from_str(r#"{ "result": "error", "error-type": "invalid-key" }"#).unwrap();
        let error = parse_exchange_rate(answer, Service::ExchangerateApi, StatusCode::FORBIDDEN)
            .unwrap_err();
        assert_eq!(error.to_string(), "ExchangeRate-API refused the API key");
    }

    #[test]
    fn unirate_takes_numbers_written_as_strings() {
        let answer: UniRateAnswer = serde_json::from_str(
            r#"{ "base": "USD", "rates": { "CAD": "1.41", "EUR": 0.9, "X": null } }"#,
        )
        .unwrap();
        let rates = parse_unirate(answer, StatusCode::OK, 100).unwrap();
        assert_eq!(rates.rates.len(), 2);
        assert_eq!(rates.rates["CAD"], 1.41);
        assert_eq!(rates.next, 100 + DAY);
    }

    #[test]
    fn a_keyed_service_without_a_key_is_refused() {
        let config: CurrencyConfig =
            serde_json::from_str(r#"{ "service": "exchangerate-api" }"#).unwrap();
        let error = source(&config).err().expect("no key must be an error");
        assert!(error.to_string().contains("currency.api-key"));
    }

    /// For whoever touches this file: the open endpoint, for real.
    #[test]
    #[ignore = "reaches the network"]
    fn the_open_endpoint_answers() {
        let rates = source(&CurrencyConfig::default())
            .unwrap()
            .fetch(When::Latest)
            .unwrap();
        assert!(rates.rates.len() > 100);
        assert_eq!(rates.rates["USD"], 1.0);
    }
}
