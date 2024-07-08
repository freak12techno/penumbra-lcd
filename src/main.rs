#[macro_use]
extern crate rocket;

use rocket::serde::json::{json, Value};
use rocket::State;
use clap::Parser;

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
use penumbra_stake::{IdentityKey, Uptime};

use tonic::transport::{Channel, ClientTlsConfig};


#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    node: String,

    #[arg(short, long, default_value_t = 8000)]
    port: i32,
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
                staking_params,
                slashing_params,
                signing_info,
            ],
        )
}
