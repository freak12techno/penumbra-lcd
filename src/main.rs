#[macro_use]
extern crate rocket;

use rocket::serde::json::{json, Value};
use rocket::State;
use clap::Parser;
use futures::TryStreamExt;

use penumbra_proto::{
    core::app::v1::{
        query_service_client::QueryServiceClient as AppQueryServiceClient, AppParametersRequest, AppParameters,
    },
    core::component::sct::v1::{
        query_service_client::QueryServiceClient as SctQueryServiceClient, EpochByHeightRequest,
    },
    core::component::stake::v1::{
        query_service_client::QueryServiceClient as StakeQueryServiceClient,
        ValidatorInfoRequest,
        ValidatorUptimeRequest,        
    },
    util::tendermint_proxy::v1::{
        tendermint_proxy_service_client::TendermintProxyServiceClient, GetStatusRequest,
    },
};
use penumbra_stake::{
    IdentityKey, Uptime,
    validator::{self, Info, Status, Validator, ValidatorToml, BondingState, State as ValidatorState},
};

use tonic::transport::{Channel, ClientTlsConfig};


#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    node: String,

    #[arg(short, long, default_value_t = 8000)]
    port: i32,
}

#[get("/cosmos/staking/v1beta1/validators?<status>")]
async fn validators(status: Option<String>, args: &State<Args>) -> Value {
    let channel = Channel::from_shared(args.node.to_string())
        .unwrap()
        .tls_config(ClientTlsConfig::new())
        .unwrap()
        .connect()
        .await
        .unwrap();

    let mut client = StakeQueryServiceClient::new(channel);

    let validators: Vec<validator::Info> = client
        .validator_info(ValidatorInfoRequest {
            show_inactive: true,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner()
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
        .into_iter()
        .map(TryInto::try_into)
        .collect::<Result<Vec<validator::Info>, _>>()
        .unwrap();

    let mut result: Vec<_> = vec![];
    for validator in validators {
        let validator_status = match validator.status.bonding_state {
            BondingState::Bonded => "BOND_STATUS_BONDED",
            BondingState::Unbonding { unbonds_at_height: _ } => "BOND_STATUS_UNBONDING",
            BondingState::Unbonded => "BOND_STATUS_UNBONDED",
        };

        if !status.is_none() && status != Some(validator_status.to_owned()) {
            continue;
        }

        result.push(json!({
            "operator_address": validator.validator.identity_key.to_string(),
            "consensus_pubkey": {
                "@type": "/cosmos.crypto.ed25519.PubKey",
                "key": base64::encode(validator.validator.consensus_key.to_bytes()),
            },
            "jailed": validator.status.state == ValidatorState::Jailed,
            "status": validator_status,
            "tokens": validator.status.voting_power.value().to_string(),
            "delegator_shares": validator.status.voting_power.value().to_string(),
            "description": {
                "moniker": validator.validator.name,
                "identity": "",
                "website": validator.validator.website,
                "security_contact": "",
                "details": validator.validator.description,
            },
            "unbonding_height": "0", // TODO
            "unbonding_time": "1970-01-01T00:00:00Z", // TODO
            "commission": {
                "commission_rates": {
                    "rate": "0.05",
                    "max_rate": "1.0",
                    "max_change_rate": "1.0"
                },
                "update_time": "2023-08-04T06:00:00.000000000Z" // TODO
            },
            "min_self_delegation": "0"
        }));
    }

    json!({
        "validators": result,
        "pagination": {
            "next_key": null,
            "total": result.len().to_string()
        }
    })
}

#[get("/cosmos/slashing/v1beta1/params")]
async fn slashing_params(args: &State<Args>) -> Value {
    let channel = Channel::from_shared(args.node.to_string())
        .unwrap()
        .tls_config(ClientTlsConfig::new())
        .unwrap()
        .connect()
        .await
        .unwrap();

    let mut client = AppQueryServiceClient::new(channel);
    let params: AppParameters = client
        .app_parameters(tonic::Request::new(AppParametersRequest {}))
        .await
        .unwrap()
        .into_inner()
        .app_parameters
        .unwrap()
        .try_into()
        .unwrap();

    let stake_params = params.stake_params.unwrap();
    let min_signed_per_window = 1.0 - (stake_params.missed_blocks_maximum as f64)
        / (stake_params.signed_blocks_window_len as f64);

    json!({
        "params": {
            "signed_blocks_window": stake_params.signed_blocks_window_len.to_string(),
            "min_signed_per_window": min_signed_per_window.to_string(),
            "downtime_jail_duration": "0s",
            "slash_fraction_double_sign": "0.0",
            "slash_fraction_downtime": "0.0",
        }
    })
}

#[get("/cosmos/staking/v1beta1/params")]
async fn staking_params(args: &State<Args>) -> Value {
    let channel = Channel::from_shared(args.node.to_string())
        .unwrap()
        .tls_config(ClientTlsConfig::new())
        .unwrap()
        .connect()
        .await
        .unwrap();

    let mut client = AppQueryServiceClient::new(channel);
    let params: AppParameters = client
        .app_parameters(tonic::Request::new(AppParametersRequest {}))
        .await
        .unwrap()
        .into_inner()
        .app_parameters
        .unwrap()
        .try_into()
        .unwrap();

    let stake_params = params.stake_params.unwrap();

    json!({
        "params": {
            "unbonding_time": "1814400s", // 21 days
            "max_validators": stake_params.active_validator_limit,
            "max_entries": 7,
            "historical_entries": 10000,
            "bond_denom": "upenumbra"
        }
    })
}


#[get("/cosmos/slashing/v1beta1/signing_infos/<identity_key>")]
async fn signing_info(identity_key: &str, args: &State<Args>) -> Value {
    let identity_key_parsed = identity_key.parse::<IdentityKey>().unwrap();

    let channel = Channel::from_shared(args.node.to_string())
        .unwrap()
        .tls_config(ClientTlsConfig::new())
        .unwrap()
        .connect()
        .await
        .unwrap();

    let mut client = StakeQueryServiceClient::new(channel);
    let uptime: Uptime = client
        .validator_uptime(ValidatorUptimeRequest {
            identity_key: Some(identity_key_parsed.into()),
        })
        .await
        .unwrap()
        .into_inner()
        .uptime
        .unwrap()
        .try_into()
        .unwrap();

    let missed_blocks = uptime.num_missed_blocks();

    json!({
        "val_signing_info": {
            "address": identity_key,
            "start_height": "0",
            "index_offset": "0",
            "jailed_until": "1970-01-01T00:00:00Z",
            "tombstoned": false,
            "missed_blocks_counter": missed_blocks.to_string()
        }
    })
}


#[launch]
fn rocket() -> _ {
    let args = Args::parse();

    rocket::build()
        .configure(rocket::Config::figment().merge(("port", args.port)))
        .manage(args)
        .mount(
            "/",
            routes![
                validators,
                staking_params,
                slashing_params,
                signing_info,
            ],
        )
}
