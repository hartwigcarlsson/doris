//! Bolagsverket's API for värdefulla datamängder: free company details by
//! organisationsnummer, used to pre-fill the company form. OAuth2 client
//! credentials; the token is cached until a minute before it expires.
//!
//! The org nr may be a personnummer: it goes only in the POST body, and
//! failure reasons never include it (`reqwest::Error::without_url`).

use doris_company::domain::{Address, LegalForm, OrgNr};
use serde::Deserialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const SCOPE: &str = "vardefulla-datamangder:read";
pub const TOKEN_URL: &str = "https://portal.api.bolagsverket.se/oauth2/token";
pub const API_URL: &str = "https://gw.api.bolagsverket.se/vardefulla-datamangder/v1";

pub struct Bolagsverket {
    http: reqwest::Client,
    token_url: String,
    api_url: String,
    client_id: String,
    client_secret: String,
    token: Mutex<Option<(String, Instant)>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub name: String,
    pub legal_form: LegalForm,
    pub address: Address,
}

#[derive(Debug)]
pub enum LookupError {
    NotFound,
    /// For the log. Never contains the org nr.
    Failed(String),
}

impl Bolagsverket {
    pub fn new(token_url: &str, api_url: &str, client_id: String, client_secret: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("the system TLS library loads");
        Self {
            http,
            token_url: token_url.to_owned(),
            api_url: api_url.trim_end_matches('/').to_owned(),
            client_id,
            client_secret,
            token: Mutex::new(None),
        }
    }

    pub async fn lookup(&self, org_nr: &OrgNr) -> Result<Found, LookupError> {
        let token = self.token().await?;
        let response = self
            .http
            .post(format!("{}/organisationer", self.api_url))
            .bearer_auth(token)
            .json(&serde_json::json!({ "identitetsbeteckning": org_nr.as_str() }))
            .send()
            .await
            .map_err(failed)?;
        match response.status() {
            reqwest::StatusCode::NOT_FOUND => return Err(LookupError::NotFound),
            reqwest::StatusCode::UNAUTHORIZED => {
                // Revoked or expired early: fetch a new token next time.
                *self.token.lock().expect("token lock") = None;
                return Err(LookupError::Failed("organisationer: HTTP 401".into()));
            }
            s if !s.is_success() => {
                return Err(LookupError::Failed(format!("organisationer: HTTP {s}")));
            }
            _ => {}
        }
        first(response.json().await.map_err(failed)?)
    }

    async fn token(&self) -> Result<String, LookupError> {
        // Copy out and drop the guard at once: a std MutexGuard held across
        // an await would make the future !Send.
        let cached = self.token.lock().expect("token lock").clone();
        if let Some((token, valid_until)) = cached
            && Instant::now() < valid_until
        {
            return Ok(token);
        }
        let response = self
            .http
            .post(&self.token_url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", &self.client_id),
                ("client_secret", &self.client_secret),
                ("scope", SCOPE),
            ])
            .send()
            .await
            .map_err(failed)?;
        if !response.status().is_success() {
            return Err(LookupError::Failed(format!(
                "token: HTTP {}",
                response.status()
            )));
        }
        let token: Token = response.json().await.map_err(failed)?;
        let valid_until = Instant::now() + Duration::from_secs(token.expires_in.saturating_sub(60));
        *self.token.lock().expect("token lock") = Some((token.access_token.clone(), valid_until));
        Ok(token.access_token)
    }
}

fn failed(err: reqwest::Error) -> LookupError {
    LookupError::Failed(err.without_url().to_string())
}

/// Bolagsverket's organisationsform code. Unknown codes become `Other`.
fn legal_form(code: &str) -> LegalForm {
    match code {
        "AB" => LegalForm::Aktiebolag,
        "HB" => LegalForm::Handelsbolag,
        "KB" => LegalForm::Kommanditbolag,
        "E" => LegalForm::EnskildFirma,
        "EK" => LegalForm::EkonomiskForening,
        "I" => LegalForm::IdeellForening,
        "S" => LegalForm::Stiftelse,
        _ => LegalForm::Other,
    }
}

fn first(response: Organisationer) -> Result<Found, LookupError> {
    let org = response
        .organisationer
        .into_iter()
        .next()
        .ok_or(LookupError::NotFound)?;
    let names = org
        .organisationsnamn
        .and_then(|n| n.organisationsnamn_lista)
        .unwrap_or_default();
    let named = |n: &&Name| n.namn.as_deref().is_some_and(|s| !s.is_empty());
    let name = names
        .iter()
        .filter(named)
        .find(|n| {
            n.organisationsnamntyp
                .as_ref()
                .is_some_and(|t| t.kod.as_deref() == Some("FORETAGSNAMN"))
        })
        .or(names.iter().find(named))
        .and_then(|n| n.namn.clone())
        .unwrap_or_default();
    let postal = org
        .postadress_organisation
        .and_then(|p| p.postadress)
        .unwrap_or_default();
    // Over-long fields from the registry are dropped; the user can type them.
    let address = Address::parse(
        postal.utdelningsadress.as_deref().unwrap_or(""),
        postal.postnummer.as_deref().unwrap_or(""),
        postal.postort.as_deref().unwrap_or(""),
    )
    .unwrap_or_default();
    Ok(Found {
        name,
        legal_form: org
            .organisationsform
            .and_then(|f| f.kod)
            .map_or(LegalForm::Other, |kod| legal_form(&kod)),
        address,
    })
}

#[derive(Deserialize)]
struct Token {
    access_token: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct Organisationer {
    organisationer: Vec<Organisation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Organisation {
    organisationsnamn: Option<Names>,
    organisationsform: Option<Code>,
    postadress_organisation: Option<PostalWrapper>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Names {
    organisationsnamn_lista: Option<Vec<Name>>,
}

#[derive(Deserialize)]
struct Name {
    namn: Option<String>,
    organisationsnamntyp: Option<Code>,
}

#[derive(Deserialize)]
struct Code {
    kod: Option<String>,
}

#[derive(Deserialize)]
struct PostalWrapper {
    postadress: Option<PostalAddress>,
}

#[derive(Default, Deserialize)]
struct PostalAddress {
    utdelningsadress: Option<String>,
    postnummer: Option<String>,
    postort: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const ERICSSON: &str = r#"{ "organisationer": [ {
        "avregistreradOrganisation": null,
        "organisationsform": { "kod": "AB", "klartext": "Aktiebolag", "dataproducent": "Bolagsverket", "fel": null },
        "organisationsidentitet": { "identitetsbeteckning": "5560160680", "typ": { "kod": "ORGNR", "klartext": "Organisationsnummer" } },
        "organisationsnamn": { "dataproducent": "Bolagsverket", "fel": null, "organisationsnamnLista": [
            { "namn": "Ericsson", "organisationsnamntyp": { "kod": "BIFIRMA", "klartext": "Bifirma" } },
            { "namn": "Telefonaktiebolaget LM Ericsson", "organisationsnamntyp": { "kod": "FORETAGSNAMN", "klartext": "Företagsnamn" } }
        ] },
        "postadressOrganisation": {
            "postadress": { "postnummer": "16483", "coAdress": null, "land": null, "postort": "STOCKHOLM", "utdelningsadress": null },
            "dataproducent": "Bolagsverket", "fel": null
        }
    } ] }"#;

    #[test]
    fn maps_name_legal_form_and_address() {
        let found = first(serde_json::from_str(ERICSSON).unwrap()).unwrap();
        assert_eq!(found.name, "Telefonaktiebolaget LM Ericsson");
        assert_eq!(found.legal_form, LegalForm::Aktiebolag);
        assert_eq!(found.address.street, None);
        assert_eq!(found.address.postal_code.as_deref(), Some("16483"));
        assert_eq!(found.address.city.as_deref(), Some("STOCKHOLM"));
    }

    #[test]
    fn parses_a_response_with_nulls() {
        let json = r#"{ "organisationer": [ { "organisationsnamn": null,
            "organisationsform": null, "postadressOrganisation": { "postadress": null } } ] }"#;
        let found = first(serde_json::from_str(json).unwrap()).unwrap();
        assert_eq!(
            found,
            Found {
                name: String::new(),
                legal_form: LegalForm::Other,
                address: Address::default()
            }
        );
    }

    #[test]
    fn null_leaves_do_not_fail_the_lookup() {
        let json = r#"{ "organisationer": [ {
            "organisationsform": { "kod": null },
            "organisationsnamn": { "organisationsnamnLista": [
                { "namn": null, "organisationsnamntyp": { "kod": "FORETAGSNAMN" } },
                { "namn": "Exempel AB", "organisationsnamntyp": { "kod": null } }
            ] } } ] }"#;
        let found = first(serde_json::from_str(json).unwrap()).unwrap();
        assert_eq!(found.name, "Exempel AB");
        assert_eq!(found.legal_form, LegalForm::Other);
    }

    #[test]
    fn an_empty_list_is_not_found() {
        assert!(matches!(
            first(serde_json::from_str(r#"{ "organisationer": [] }"#).unwrap()),
            Err(LookupError::NotFound)
        ));
    }

    #[test]
    fn organisationsform_codes_map_to_legal_forms() {
        for (code, form) in [
            ("AB", LegalForm::Aktiebolag),
            ("HB", LegalForm::Handelsbolag),
            ("KB", LegalForm::Kommanditbolag),
            ("E", LegalForm::EnskildFirma),
            ("EK", LegalForm::EkonomiskForening),
            ("I", LegalForm::IdeellForening),
            ("S", LegalForm::Stiftelse),
            ("BRF", LegalForm::Other),
        ] {
            assert_eq!(legal_form(code), form, "{code}");
        }
    }
}
