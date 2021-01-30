use crate::event::{BlankNetwork, Event, EventType};
use crate::{api::start_api, event::Notification, Error, Result};
use eventually::Aggregate;
use serde::__private::de::InPlaceSeed;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Default)]
pub struct IdentityManager<'a> {
    identities: HashMap<NetworkAddress, Vec<FieldStatus>>,
    lookup_addresses: HashMap<&'a IdentityField, HashSet<&'a NetworkAddress>>,
}

// TODO: Should logs be printed if users are not found?
impl<'a> IdentityManager<'a> {
    fn new() -> Self {
        IdentityManager {
            identities: HashMap::new(),
            lookup_addresses: HashMap::new(),
        }
    }
    fn insert_identity(&'a mut self, identity: IdentityState) {
        // Insert identity.
        let (net_address, fields) = (identity.net_address, identity.fields);
        self.identities.insert(net_address.clone(), fields);

        // Acquire references to the key/value from within the map. Unwrapping
        // is fine here since the value was just inserted.
        let (net_address, fields) = self.identities.get_key_value(&net_address).unwrap();

        // Create fast lookup tables.
        for field in fields {
            self.lookup_addresses
                .entry(&field.field)
                .and_modify(|active_addresses| {
                    active_addresses.insert(net_address);
                })
                .or_insert(vec![net_address].into_iter().collect());
        }
    }
    pub fn update_field(
        &mut self,
        net_address: &NetworkAddress,
        field_status: FieldStatus,
    ) -> Result<()> {
        self.identities
            .get_mut(net_address)
            .ok_or(anyhow!("network address not found"))
            .and_then(|statuses| {
                statuses
                    .iter_mut()
                    .find(|status| status.field == field_status.field)
                    .map(|status| {
                        *status = field_status;
                        ()
                    })
                    .ok_or(anyhow!("field not found"))
            })
    }
    fn lookup_field_status(
        &self,
        net_address: &NetworkAddress,
        field: &IdentityField,
    ) -> Option<&FieldStatus> {
        self.identities
            .get(net_address)
            .map(|statuses| statuses.iter().find(|status| &status.field == field))
            // Unpack `Option<Option<T>>` to `Option<T>`
            .and_then(|status| status)
    }
    fn lookup_field_status_mut(
        &mut self,
        net_address: &NetworkAddress,
        field: &IdentityField,
    ) -> Option<&mut FieldStatus> {
        self.identities
            .get_mut(net_address)
            .map(|statuses| statuses.iter_mut().find(|status| &status.field == field))
            // Unpack `Option<Option<T>>` to `Option<T>`
            .and_then(|status| status)
    }
    // Lookup all addresses which contain the specified field.
    fn lookup_addresses(&self, field: &IdentityField) -> Option<Vec<&NetworkAddress>> {
        self.lookup_addresses
            .get(field)
            .map(|addresses| addresses.iter().map(|address| *address).collect())
    }
    pub fn lookup_full_state(&self, net_address: &NetworkAddress) -> Option<IdentityState> {
        self.identities
            .get(net_address)
            .map(|fields| IdentityState {
                net_address: net_address.clone(),
                fields: fields.clone(),
            })
    }
    pub fn verify_message(
        &self,
        field: &IdentityField,
        message: &ProvidedMessage,
    ) -> Vec<VerificationOutcome> {
        let mut outcomes = vec![];

        // TODO: Reject Display Name fields. Just in case.

        // Lookup all addresses which contain the field.
        if let Some(net_addresses) = self.lookup_addresses(field) {
            // For each address, verify the field.
            for net_address in net_addresses {
                if let Some(field_status) = self.lookup_field_status(&net_address, field) {
                    // Variables must be cloned, since those are later converted
                    // into events (which requires ownership) and sent to the
                    // event store.
                    let net_address = net_address.clone();
                    let field_status = field_status.clone();

                    // Verify the message, each verified specifically based on
                    // the challenge type.
                    match field_status.challenge {
                        ChallengeStatus::ExpectMessage(ref challenge) => {
                            if challenge.status != Validity::Valid {
                                outcomes.push({
                                    if challenge.expected_message.contains(message).is_some() {
                                        VerificationOutcome {
                                            net_address: net_address,
                                            field_status: field_status,
                                        }
                                    } else {
                                        VerificationOutcome {
                                            net_address: net_address,
                                            field_status: field_status,
                                        }
                                    }
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
        };

        outcomes
    }
    // TODO: Should return Result
    pub fn set_verified(&mut self, net_address: &NetworkAddress, field: &IdentityField) -> bool {
        if let Some(field_status) = self.lookup_field_status_mut(net_address, field) {
            // TODO
            //field_status.is_valid = true;
            true
        } else {
            false
        }
    }
    // TODO: Should return Result
    pub fn is_fully_verified(&self, net_address: &NetworkAddress) -> Result<bool> {
        self.identities
            .get(net_address)
            .map(|field_statuses| field_statuses.iter().any(|status| status.is_valid()))
            .ok_or(
                Error::TargetAddressNotFound(net_address.clone(), "is_fully_verified".to_string())
                    .into(),
            )
    }
    // TODO: Should return Result
    pub fn remove_identity(&mut self, net_address: &NetworkAddress) -> bool {
        false
    }
}

#[derive(Eq, PartialEq, Hash, Clone, Debug)]
pub struct VerificationOutcome {
    pub net_address: NetworkAddress,
    pub field_status: FieldStatus,
}

#[derive(Eq, PartialEq, Hash, Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "network", content = "address")]
pub enum NetworkAddress {
    #[serde(rename = "polkadot")]
    Polkadot(IdentityAddress),
    #[serde(rename = "kusama")]
    Kusama(IdentityAddress),
}

impl NetworkAddress {
    pub fn from(network: BlankNetwork, address: IdentityAddress) -> Self {
        match network {
            BlankNetwork::Polkadot => NetworkAddress::Polkadot(address),
            BlankNetwork::Kusama => NetworkAddress::Kusama(address),
        }
    }
    pub fn net_address_str(&self) -> &str {
        match self {
            NetworkAddress::Polkadot(address) => address.0.as_str(),
            NetworkAddress::Kusama(address) => address.0.as_str(),
        }
    }
}

#[derive(Eq, PartialEq, Hash, Clone, Debug, Serialize, Deserialize)]
pub struct IdentityState {
    pub net_address: NetworkAddress,
    fields: Vec<FieldStatus>,
}

impl From<IdentityState> for Event {
    fn from(val: IdentityState) -> Self {
        EventType::IdentityState(val).into()
    }
}

impl IdentityState {
    pub fn set_validity(&mut self, target: &IdentityField, validity: Validity) -> Result<()> {
        self.fields
            .iter_mut()
            .find(|status| &status.field == target)
            .map(|status| {
                status.set_validity(validity);
                ()
            })
            .ok_or(anyhow!("Target field was not found"))
    }
}

#[derive(Eq, PartialEq, Hash, Clone, Debug, Serialize, Deserialize)]
pub struct IdentityAddress(String);

#[derive(Eq, PartialEq, Hash, Clone, Debug, Serialize, Deserialize)]
pub struct FieldStatus {
    field: IdentityField,
    is_permitted: bool,
    challenge: ChallengeStatus,
}

impl FieldStatus {
    pub fn is_valid(&self) -> bool {
        let status = match &self.challenge {
            ChallengeStatus::ExpectMessage(state) => &state.status,
            ChallengeStatus::BackAndForth(state) => {
                if state.first_check_status == Validity::Valid
                    && state.second_check_status == Validity::Valid
                {
                    return true;
                } else {
                    return false;
                }
            }
            ChallengeStatus::CheckDisplayName(state) => &state.status,
        };

        match status {
            Validity::Valid => true,
            Validity::Invalid | Validity::Unconfirmed => false,
        }
    }
    pub fn set_validity(&mut self, validity: Validity) {
        let mut status = match self.challenge {
            ChallengeStatus::ExpectMessage(ref mut state) => &mut state.status,
            ChallengeStatus::BackAndForth(ref mut state) => &mut state.first_check_status,
            ChallengeStatus::CheckDisplayName(ref mut state) => &mut state.status,
        };

        *status = validity;
    }
    // TODO: Should this maybe return `Result`?
    pub fn expected_message(&self) -> Option<&ExpectedMessage> {
        match self.challenge {
            ChallengeStatus::ExpectMessage(ref state) => Some(&state.expected_message),
            ChallengeStatus::BackAndForth(ref state) => Some(&state.expected_message),
            _ => None,
        }
    }
}

#[derive(Eq, PartialEq, Hash, Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "state")]
pub enum ChallengeStatus {
    #[serde(rename = "expect_message")]
    ExpectMessage(ExpectMessageChallenge),
    #[serde(rename = "back_and_forth")]
    BackAndForth(BackAndForthChallenge),
    #[serde(rename = "display_name_check")]
    CheckDisplayName(CheckDisplayNameChallenge),
}

#[derive(Eq, PartialEq, Hash, Clone, Debug, Serialize, Deserialize)]
pub struct ExpectMessageChallenge {
    pub expected_message: ExpectedMessage,
    pub from: IdentityField,
    pub to: IdentityField,
    pub status: Validity,
}

#[derive(Eq, PartialEq, Hash, Clone, Debug, Serialize, Deserialize)]
pub struct BackAndForthChallenge {
    pub expected_message: ExpectedMessage,
    // VERY IMPORTANT: This field MUST be skipped during serializing and MAY NO
    // be sent to the the end user via the API, since the message must be
    // explicitly received by the specified `from` address and sent back to the
    // service (`to` address).
    #[serde(skip_serializing)]
    pub expected_message_back: ExpectedMessage,
    pub from: IdentityField,
    pub to: IdentityField,
    pub first_check_status: Validity,
    pub second_check_status: Validity,
}

#[derive(Eq, PartialEq, Hash, Clone, Debug, Serialize, Deserialize)]
pub struct CheckDisplayNameChallenge {
    pub status: Validity,
    pub similarities: Option<Vec<DisplayName>>,
}

#[derive(Eq, PartialEq, Hash, Clone, Debug, Serialize, Deserialize)]
pub enum Validity {
    #[serde(rename = "valid")]
    Valid,
    #[serde(rename = "invalid")]
    Invalid,
    #[serde(rename = "unconfirmed")]
    Unconfirmed,
}

#[derive(Eq, PartialEq, Hash, Clone, Debug, Serialize, Deserialize)]
pub struct DisplayName(String);

#[derive(Eq, PartialEq, Hash, Clone, Debug, Serialize, Deserialize)]
pub struct FieldAddress(String);

#[derive(Eq, PartialEq, Hash, Clone, Debug, Serialize, Deserialize)]
pub struct ExpectedMessage(String);

#[derive(Eq, PartialEq, Hash, Clone, Debug, Serialize, Deserialize)]
pub struct ProvidedMessage {
    parts: Vec<ProvidedMessagePart>,
}

#[derive(Eq, PartialEq, Hash, Clone, Debug, Serialize, Deserialize)]
pub struct ProvidedMessagePart(String);

impl ExpectedMessage {
    fn contains<'a>(&self, message: &'a ProvidedMessage) -> Option<&'a ProvidedMessagePart> {
        for part in &message.parts {
            if self.0.contains(&part.0) {
                return Some(part);
            }
        }

        None
    }
}

#[derive(Eq, PartialEq, Hash, Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "address")]
pub enum IdentityField {
    #[serde(rename = "legal_name")]
    LegalName(FieldAddress),
    #[serde(rename = "display_name")]
    DisplayName(FieldAddress),
    #[serde(rename = "email")]
    Email(FieldAddress),
    #[serde(rename = "web")]
    Web(FieldAddress),
    #[serde(rename = "twitter")]
    Twitter(FieldAddress),
    #[serde(rename = "matrix")]
    Matrix(FieldAddress),
    #[serde(rename = "pgpFingerprint")]
    PGPFingerprint(FieldAddress),
    #[serde(rename = "image")]
    /// NOTE: Currently unsupported.
    Image,
    #[serde(rename = "additional")]
    /// NOTE: Currently unsupported.
    Additional,
}

#[test]
fn print_identity_info() {
    let state = IdentityState {
        net_address: NetworkAddress::Polkadot(IdentityAddress(
            "15MUBwP6dyVw5CXF9PjSSv7SdXQuDSwjX86v1kBodCSWVR7cw".to_string(),
        )),
        fields: vec![
            FieldStatus {
                field: IdentityField::Matrix(FieldAddress("@alice:matrix.org".to_string())),
                challenge: ChallengeStatus::ExpectMessage(ExpectMessageChallenge {
                    expected_message: ExpectedMessage("1127233905".to_string()),
                    from: IdentityField::Matrix(FieldAddress("@alice:matrix.org".to_string())),
                    to: IdentityField::Matrix(FieldAddress("@registrar:matrix.org".to_string())),
                    status: Validity::Valid,
                }),
            },
            FieldStatus {
                field: IdentityField::Email(FieldAddress("alice@email.com".to_string())),
                challenge: ChallengeStatus::BackAndForth(BackAndForthChallenge {
                    expected_message: ExpectedMessage("6861321088".to_string()),
                    from: IdentityField::Email(FieldAddress("alice@email.com".to_string())),
                    to: IdentityField::Email(FieldAddress("registrar@web3.foundation".to_string())),
                    first_check_status: Validity::Unconfirmed,
                    second_check_status: Validity::Unconfirmed,
                }),
            },
            FieldStatus {
                field: IdentityField::DisplayName(FieldAddress("Alice in Wonderland".to_string())),
                challenge: ChallengeStatus::CheckDisplayName(CheckDisplayNameChallenge {
                    status: Validity::Valid,
                    similarities: None,
                }),
            },
        ],
    };

    use crate::event::{Notification, StateWrapper};

    println!(
        "{}",
        serde_json::to_string(&StateWrapper {
            state: state,
            notifications: vec![Notification::Success(
                "The Matrix account has been verified".to_string()
            ),]
        })
        .unwrap()
    )
}
