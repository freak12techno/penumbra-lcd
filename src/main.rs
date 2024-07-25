#[macro_use]
extern crate rocket;

// This change will break the cache

use penumbra_proto::util::tendermint_proxy::v1::SyncInfo;
use rocket::response::status;
use rocket::serde::json::{Json, json, Value};
use rocket::State;
use clap::Parser;
use futures::TryStreamExt;
use std::net::IpAddr;
use chrono::DateTime;

use penumbra_proto::{
    core::app::v1::{
        query_service_client::QueryServiceClient as AppQueryServiceClient,
        AppParametersRequest,
        AppParameters,
    },
    core::component::stake::v1::{
        query_service_client::QueryServiceClient as StakeQueryServiceClient,
        ValidatorInfoRequest,
        ValidatorUptimeRequest,        
    },
    core::component::governance::v1::{
        query_service_client::QueryServiceClient as GovernanceQueryServiceClient,
        ProposalDataRequest,
        ProposalDataResponse,
        ProposalListRequest,
        ProposalListResponse,
        ValidatorVotesRequest,
        ValidatorVotesResponse,
        proposal_state::State as ProposalState,
        proposal_outcome::Outcome,
        proposal_state::Finished,
        Proposal,
    },
    util::tendermint_proxy::v1::{
        tendermint_proxy_service_client::TendermintProxyServiceClient,
        GetStatusRequest,
        GetStatusResponse,
        GetBlockByHeightRequest,
        GetBlockByHeightResponse,
    }
};
use penumbra_stake::{
    IdentityKey, Uptime,
    validator::{self, BondingState, State as ValidatorState},
};

use tonic::transport::{Channel, ClientTlsConfig};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    node: String,

    #[arg(short, long, default_value_t = 8000)]
    port: i32,

    #[arg(short, long, default_value_t = String::from("127.0.0.1"))]
    bind: String,
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

async fn get_sync_info(channel: Channel) -> SyncInfo {
    let mut tendermint_client = TendermintProxyServiceClient::new(channel.clone());
    let status_data: GetStatusResponse = tendermint_client
        .get_status(GetStatusRequest { })
        .await
        .unwrap()
        .into_inner();

    status_data.sync_info.unwrap()
}

async fn get_block_time(channel: Channel, latest_block_height: i64, latest_block_time: f64) -> f64 {
    let mut tendermint_client = TendermintProxyServiceClient::new(channel.clone());

    let older_block_data: GetBlockByHeightResponse = tendermint_client
        .get_block_by_height(GetBlockByHeightRequest { height: latest_block_height - 100 })
        .await
        .unwrap()
        .into_inner();

    let older_block_header = older_block_data.block.unwrap().header.unwrap();
    let older_block_height = older_block_header.height;
    let older_block_time: f64 = older_block_header.time.unwrap().seconds as f64;

    let time_between_blocks = latest_block_time - older_block_time;
    let blocks_diff = latest_block_height - older_block_height;

    time_between_blocks / (blocks_diff as f64)
}


fn map_proposal(
    proposal_id: u64,
    proposal: Proposal,
    state: ProposalState,
    latest_block_height: i64,
    latest_block_time: f64,
    start_block_height: u64,
    end_block_height: u64,
    block_time: f64,
) -> Value {
    let state = match state {
        ProposalState::Voting(_) => "PROPOSAL_STATUS_VOTING_PERIOD",
        ProposalState::Finished(Finished { outcome: Some(value) }) => {
            match value.outcome.unwrap() {
                Outcome::Passed(_) => "PROPOSAL_STATUS_PASSED",
                Outcome::Failed(_) => "PROPOSAL_STATUS_REJECTED",
                _ => "ProposalStatus_PROPOSAL_STATUS_UNSPECIFIED"
            }
        },
        _ => "ProposalStatus_PROPOSAL_STATUS_UNSPECIFIED"
    };

    let voting_start = latest_block_time
        - ((latest_block_height - (start_block_height as i64)) as f64) * block_time;
    let voting_end = latest_block_time
        - ((latest_block_height - (end_block_height as i64)) as f64) * block_time;

    json!({
        "proposal_id": proposal_id.to_string(),
        "content": {
            "@type": "penumbra.core.component.governance.v1.Signaling",
            "title": proposal.title,
            "description": proposal.description,
        },
        "status": state,
        "final_tally_result": {
            "yes": "0",
            "abstain": "0",
            "no": "0",
            "no_with_veto": "0"
        },
        "submit_time": "1970-01-01T00:00:00.000Z",
        "deposit_end_time": "1970-01-01T00:00:00.000Z",
        "total_deposit": [],
        "voting_start_time": DateTime::from_timestamp(voting_start as i64, 0),
        "voting_end_time": DateTime::from_timestamp(voting_end as i64, 0),
    })
}

#[get("/cosmos/gov/v1beta1/proposals/<proposal_id>")]
async fn proposal(proposal_id: u64, args: &State<Args>) -> Value {
    let channel = Channel::from_shared(args.node.to_string())
        .unwrap()
        .tls_config(ClientTlsConfig::new())
        .unwrap()
        .connect()
        .await
        .unwrap();

    let mut client = GovernanceQueryServiceClient::new(channel.clone());
    let proposal_data: ProposalDataResponse = client
        .proposal_data(ProposalDataRequest { proposal_id: proposal_id })
        .await
        .unwrap()
        .into_inner();

    let sync_info = get_sync_info(channel.clone()).await;
    let latest_block_height: i64 = (sync_info.latest_block_height) as i64;
    let latest_block_time: f64 = sync_info.latest_block_time.unwrap().seconds as f64;
    let block_time = get_block_time(channel.clone(), latest_block_height, latest_block_time).await;

    let proposal = map_proposal(
        proposal_id,
        proposal_data.proposal.unwrap(),
        proposal_data.state.unwrap().state.unwrap(),
        latest_block_height,
        latest_block_time,
        proposal_data.start_block_height,
        proposal_data.end_block_height,
        block_time,
    );

    json!({
        "proposal": proposal,
    })
}

#[get("/cosmos/gov/v1beta1/proposals")]
async fn proposals(args: &State<Args>) -> Value {
    let channel = Channel::from_shared(args.node.to_string())
        .unwrap()
        .tls_config(ClientTlsConfig::new())
        .unwrap()
        .connect()
        .await
        .unwrap();

    let mut client = GovernanceQueryServiceClient::new(channel.clone());

    let proposals: Vec<ProposalListResponse> = client
        .proposal_list(ProposalListRequest { inactive: true })
        .await
        .unwrap()
        .into_inner()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();

    let sync_info = get_sync_info(channel.clone()).await;
    let latest_block_height: i64 = (sync_info.latest_block_height) as i64;
    let latest_block_time: f64 = sync_info.latest_block_time.unwrap().seconds as f64;
    let block_time = get_block_time(channel.clone(), latest_block_height, latest_block_time).await;

    let mut response: Vec<Value> = vec![];

    for proposal in proposals {
        let proposal_unwrapped = proposal.proposal.unwrap();
        let proposal_mapped = map_proposal(
            proposal_unwrapped.id,
            proposal_unwrapped,
            proposal.state.unwrap().state.unwrap(),
            latest_block_height,
            latest_block_time,
            proposal.start_block_height,
            proposal.end_block_height,
            block_time,
        );

        response.push(proposal_mapped);
    }

    json!({
        "proposals": response,
        "pagination": {
            "next_key": null,
            "total": response.len().to_string(),
        }
    })
}


async fn get_vote(voter: &str, proposal_id: u64, args: &State<Args>) -> status::Custom<Json<Value>> {
  let channel = Channel::from_shared(args.node.to_string())
      .unwrap()
      .tls_config(ClientTlsConfig::new())
      .unwrap()
      .connect()
      .await
      .unwrap();

  let mut client = GovernanceQueryServiceClient::new(channel.clone());
  let votes_data: Vec<ValidatorVotesResponse> = client
      .validator_votes(ValidatorVotesRequest { proposal_id: proposal_id })
      .await
      .unwrap()
      .into_inner()
      .try_collect::<Vec<_>>()
      .await
      .unwrap();

  let validator_vote = votes_data
      .iter()
      .find(|&vote| {
          let identity: IdentityKey = vote.clone().identity_key.unwrap().try_into().unwrap();
          identity.to_string() == voter
      });

  let response = match validator_vote {
      None => {
          status::Custom(rocket::http::Status::BadRequest, Json(json!({
              "code": 3,
              "message": format!("voter: {} not found for proposal: {}", voter, proposal_id),
              "details": []
          })))
      },
      Some(vote) => {
          match &vote.vote {
              None => {
                  status::Custom(rocket::http::Status::BadRequest, Json(json!({
                      "code": 3,
                      "message": format!("voter: {} not found for proposal: {}", voter, proposal_id),
                      "details": []
                  })))
              },
              Some(option) => {
                  let vote = match option.vote {
                      3 => "VOTE_OPTION_NO",
                      2 => "VOTE_OPTION_YES",
                      1 => "VOTE_OPTION_YES",
                      _ => "VOTE_OPTION_UNSPECIFIED"
                  };

                  status::Custom(rocket::http::Status::Ok, Json(json!({
                      "vote": {
                        "proposal_id": proposal_id.to_string(),
                        "voter": voter,
                        "option": vote,
                        "options": [
                          {
                            "option": vote,
                            "weight": "1.000000000000000000"
                          }
                        ]
                      }
                  })))
              }
          }
      }
  };

  response
}

#[get("/cosmos/gov/v1beta1/proposals/<proposal_id>/votes/<voter>")]
async fn proposal_vote(proposal_id: u64, voter: &str, args: &State<Args>) -> status::Custom<Json<Value>> {
    get_vote(voter, proposal_id, args).await
}


#[launch]
fn rocket() -> _ {
    let args = Args::parse();

    let ip_addr: IpAddr = args.bind.parse().expect("Invalid IP address format");

    rocket::build()
        .configure(rocket::Config::figment()
                   .merge(("port", args.port))
                   .merge(("address", ip_addr))
        )
        .manage(args)
        .mount(
            "/",
            routes![
                validators,
                staking_params,
                slashing_params,
                signing_info,
                proposals,
                proposal,
                proposal_vote,
            ],
        )
}
