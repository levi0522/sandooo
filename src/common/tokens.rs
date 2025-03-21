use anyhow::Result;
use csv::StringRecord;
use ethers::abi::parse_abi;
use ethers::prelude::BaseContract;
use ethers::providers::{call_raw::RawCall, Provider, Ws};
use ethers::types::{spoof, BlockNumber, TransactionRequest, H160, U256, U64};
use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use std::{collections::HashMap, fs::OpenOptions, path::Path, str::FromStr, sync::Arc};

use crate::common::bytecode::REQUEST_BYTECODE;
use crate::common::pools::Pool;
use crate::common::utils::create_new_wallet;

#[derive(Debug, Clone)]
pub struct Token {
    pub id: i64,
    pub address: H160,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    pub pool_ids: Vec<i64>, // refers to the "id" field of Pool struct
}

impl From<StringRecord> for Token {
    fn from(record: StringRecord) -> Self {
        Self {
            id: record.get(0).unwrap().parse().unwrap(),
            address: H160::from_str(record.get(1).unwrap()).unwrap(),
            name: String::from(record.get(2).unwrap()),
            symbol: String::from(record.get(3).unwrap()),
            decimals: record.get(4).unwrap().parse().unwrap(),
            pool_ids: Vec::new(),
        }
    }
}

impl Token {
    pub fn cache_row(&self) -> (i64, String, String, String, u8) {
        (
            self.id,
            format!("{:?}", self.address),
            self.name.clone(),
            self.symbol.clone(),
            self.decimals,
        )
    }
}

// for eth_call response
#[derive(Debug, Clone)]
pub struct TokenInfo {
    pub address: H160,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
}

pub async fn load_all_tokens(
    provider: &Arc<Provider<Ws>>,
    block_number: U64,
    pools: &Vec<Pool>,
    chunk: u64,
) -> Result<HashMap<H160, Token>> {
    let cache_file = "cache/.cached-tokens.csv";
    let file_path = Path::new(cache_file);
    let file_exists = file_path.exists();
    let file = OpenOptions::new()
        .write(true)
        .append(true)
        .create(true)
        .open(file_path)
        .unwrap();
    let mut writer = csv::Writer::from_writer(file);

    let mut tokens_map: HashMap<H160, Token> = HashMap::new();
    let mut token_id = 0;

    if file_exists {
        let mut reader = csv::Reader::from_path(file_path)?;

        for row in reader.records() {
            let row = row.unwrap();
            let token = Token::from(row);
            tokens_map.insert(token.address, token);
            token_id += 1;
        }
    } else {
        writer.write_record(&["id", "address", "name", "symbol", "decimals"])?;
    }

    let mut pool_processed = 0;
    let mut pool_range = Vec::new();

    loop {
        let start_idx = 1 + pool_processed;
        let mut end_idx = start_idx + chunk - 1;
        if end_idx > pools.len() as u64 {
            end_idx = pools.len() as u64;
            pool_range.push((start_idx, end_idx));
            break;
        }
        pool_range.push((start_idx, end_idx));
        pool_processed += chunk;
    }
    info!("Pool range: {:?}", pool_range);

    let pb = ProgressBar::new(pool_range.len() as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}",
        )
        .unwrap()
        .progress_chars("##-"),
    );

    let new_token_id = token_id;

    for (start_idx, end_idx) in pool_range {
        let mut new_tokens = Vec::new(); // 用于存储本次新增的 token
        for pool in &pools[start_idx as usize..end_idx as usize] {
            for token in vec![pool.token0, pool.token1] {
                if !tokens_map.contains_key(&token) {
                    match get_token_info(provider, block_number.into(), token).await {
                        Ok(token_info) => {
                            let new_token = Token {
                                id: token_id,
                                address: token,
                                name: token_info.name,
                                symbol: token_info.symbol,
                                decimals: token_info.decimals,
                                pool_ids: Vec::new(),
                            };
                            tokens_map.insert(token, new_token.clone());
                            new_tokens.push(new_token);
                            token_id += 1;
                        }
                        _ => {}
                    }
                }
            }
        }

        // **去重后写入 CSV**
        for token in &new_tokens {
            writer.serialize(token.cache_row())?;
        }
        writer.flush()?; // 立即写入文件

        pb.inc(1);
    }

    info!("Added {:?} new tokens", token_id - new_token_id);

    Ok(tokens_map)
}

pub async fn get_token_info(
    provider: &Arc<Provider<Ws>>,
    block_number: BlockNumber,
    token_address: H160,
) -> Result<TokenInfo> {
    let owner = create_new_wallet().1;

    let mut state = spoof::state();
    state.account(owner).balance(U256::MAX).nonce(0.into());

    let request_address = create_new_wallet().1;
    state
        .account(request_address)
        .code((*REQUEST_BYTECODE).clone());

    let request_abi = BaseContract::from(parse_abi(&[
        "function getTokenInfo(address) external returns (string,string,uint8,uint256)",
    ])?);
    let calldata = request_abi.encode("getTokenInfo", token_address)?;

    let gas_price = U256::from(1000)
        .checked_mul(U256::from(10).pow(U256::from(9)))
        .unwrap();
    let tx = TransactionRequest::default()
        .from(owner)
        .to(request_address)
        .value(U256::zero())
        .data(calldata.0)
        .nonce(U256::zero())
        .gas(5000000)
        .gas_price(gas_price)
        .chain_id(1)
        .into();
    let result = provider
        .call_raw(&tx)
        .state(&state)
        .block(block_number.into())
        .await?;
    let out: (String, String, u8, U256) = request_abi.decode_output("getTokenInfo", result)?;
    let token_info = TokenInfo {
        address: token_address,
        name: out.0,
        symbol: out.1,
        decimals: out.2,
    };
    Ok(token_info)
}

pub async fn get_token_info_wrapper(
    provider: Arc<Provider<Ws>>,
    block: BlockNumber,
    token_address: H160,
) -> Result<TokenInfo> {
    get_token_info(&provider, block, token_address).await
}

pub async fn get_token_info_multi(
    provider: Arc<Provider<Ws>>,
    block: BlockNumber,
    tokens: &Vec<H160>,
) -> Result<HashMap<H160, TokenInfo>> {
    let mut requests = Vec::new();
    for token in tokens {
        let req = tokio::task::spawn(get_token_info_wrapper(
            provider.clone(),
            block.clone(),
            *token,
        ));
        requests.push(req);
    }
    let results = futures::future::join_all(requests).await;

    let mut token_info = HashMap::new();
    for i in 0..tokens.len() {
        let token = tokens[i];
        let result = &results[i];
        match result {
            Ok(result) => {
                if let Ok(info) = result {
                    token_info.insert(token, info.clone());
                }
            }
            _ => {}
        };
    }

    Ok(token_info)
}
