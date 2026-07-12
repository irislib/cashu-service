use anyhow::{bail, Context, Result};
use cdk::cdk_database::WalletDatabase;
use cdk::lightning_invoice::Bolt11Invoice;
use cdk::mint_url::MintUrl;
use cdk::nuts::{CurrencyUnit, MeltQuoteState, MintQuoteState, PaymentMethod, Proof, State, Token};
use cdk::wallet::{
    types::ProofInfo, ReceiveOptions, SendOptions, WalletRepository, WalletRepositoryBuilder,
};
use cdk::Amount;
use cdk_sqlite::WalletSqliteDatabase;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;
use uuid::Uuid;

use crate::helper::{
    CashuLightningPayment, CashuMintBalance, CashuReceivedPayment, CashuSentPayment,
};

pub const CASHU_WALLET_SEED_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CashuWalletSeedFile {
    pub version: u32,
    pub seed_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashuWalletEntry {
    pub mint_url: String,
    pub unit: String,
    pub balance: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashuUnitTotal {
    pub unit: String,
    pub balance: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CashuWalletOverview {
    pub totals: Vec<CashuUnitTotal>,
    pub entries: Vec<CashuWalletEntry>,
    pub warnings: Vec<String>,
    pub legacy_state_detected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashuTopupQuote {
    pub mint_url: String,
    pub unit: String,
    pub amount: u64,
    pub quote_id: String,
    pub payment_request: String,
    pub expiry_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CashuCrossMintTransferRequest {
    /// Caller-owned idempotency key. Reusing it resumes or returns the same transfer.
    pub transfer_id: String,
    /// Caller-selected and caller-approved source mint.
    pub source_mint_url: String,
    /// Caller-selected and caller-approved destination mint.
    pub destination_mint_url: String,
    pub amount_sat: u64,
    /// Maximum total source-side fee, including the melt reserve and wallet input fees.
    pub max_fee_sat: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CashuCrossMintTransfer {
    pub transfer_id: String,
    pub source_mint_url: String,
    pub destination_mint_url: String,
    pub unit: String,
    pub amount_sat: u64,
    pub max_fee_sat: u64,
    pub melt_fee_reserve_sat: u64,
    pub wallet_fee_sat: u64,
    pub fee_paid_sat: u64,
    pub source_melt_quote_id: String,
    pub destination_mint_quote_id: String,
    pub destination_balance_before_sat: u64,
    pub destination_balance_after_sat: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CashuCrossMintTransferSaga {
    version: u32,
    request: CashuCrossMintTransferRequest,
    destination_balance_before_sat: u64,
    destination_mint_quote_id: Option<String>,
    destination_payment_request: Option<String>,
    source_melt_quote_id: Option<String>,
    melt_fee_reserve_sat: Option<u64>,
    wallet_fee_sat: Option<u64>,
    fee_paid_sat: Option<u64>,
    result: Option<CashuCrossMintTransfer>,
}

const K_WALLET_ACTIVITY_PRIMARY_NAMESPACE: &str = "iris_wallet";
const K_WALLET_ACTIVITY_SECONDARY_NAMESPACE: &str = "activity";
const K_WALLET_ACTIVITY_KEY: &str = "entries";
const K_MAX_WALLET_ACTIVITY_ENTRIES: usize = 200;
const K_CROSS_MINT_TRANSFER_NAMESPACE: &str = "cross_mint_transfer";
const K_CROSS_MINT_TRANSFER_VERSION: u32 = 1;
static CROSS_MINT_TRANSFER_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CashuWalletActivityKind {
    TopUp,
    LightningPayment,
    TokenSend,
    TokenReceive,
    ChannelCollect,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CashuWalletActivityStatus {
    Pending,
    Complete,
    Reclaimed,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CashuWalletActivityEntry {
    pub id: String,
    pub kind: CashuWalletActivityKind,
    pub status: CashuWalletActivityStatus,
    pub mint_url: String,
    pub unit: String,
    pub amount_sat: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_sat: Option<u64>,
    pub created_at_unix: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_unix: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_request: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

pub fn cashu_wallet_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("cashu")
}

pub fn cashu_wallet_db_path(data_dir: &Path) -> PathBuf {
    cashu_wallet_dir(data_dir).join("wallet.sqlite")
}

pub fn cashu_wallet_seed_path(data_dir: &Path) -> PathBuf {
    cashu_wallet_dir(data_dir).join("seed.json")
}

pub fn legacy_cashu_wallet_state_path(data_dir: &Path) -> PathBuf {
    data_dir.join("cashu-wallet.json")
}

fn wallet_activity_now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn wallet_activity_id() -> String {
    let mut suffix = [0_u8; 4];
    rand::thread_rng().fill_bytes(&mut suffix);
    format!("{}-{}", wallet_activity_now_unix(), hex::encode(suffix))
}

fn sort_wallet_activity_entries(entries: &mut Vec<CashuWalletActivityEntry>) {
    entries.sort_by(|left, right| {
        right
            .created_at_unix
            .cmp(&left.created_at_unix)
            .then_with(|| right.id.cmp(&left.id))
    });
    if entries.len() > K_MAX_WALLET_ACTIVITY_ENTRIES {
        entries.truncate(K_MAX_WALLET_ACTIVITY_ENTRIES);
    }
}

async fn open_wallet_localstore(data_dir: &Path) -> Result<Arc<WalletSqliteDatabase>> {
    fs::create_dir_all(cashu_wallet_dir(data_dir))
        .context("Failed to create Cashu wallet directory")?;
    Ok(Arc::new(
        WalletSqliteDatabase::new(cashu_wallet_db_path(data_dir))
            .await
            .context("Failed to open Cashu wallet database")?,
    ))
}

async fn load_wallet_activity_entries_from_store(
    localstore: &WalletSqliteDatabase,
) -> Result<Vec<CashuWalletActivityEntry>> {
    let stored = localstore
        .kv_read(
            K_WALLET_ACTIVITY_PRIMARY_NAMESPACE,
            K_WALLET_ACTIVITY_SECONDARY_NAMESPACE,
            K_WALLET_ACTIVITY_KEY,
        )
        .await
        .context("Failed to read Cashu wallet activity")?;
    let mut entries = match stored {
        Some(bytes) if !bytes.is_empty() => {
            serde_json::from_slice(&bytes).context("Failed to parse Cashu wallet activity")?
        }
        _ => Vec::new(),
    };
    sort_wallet_activity_entries(&mut entries);
    Ok(entries)
}

async fn save_wallet_activity_entries_to_store(
    localstore: &WalletSqliteDatabase,
    entries: &mut Vec<CashuWalletActivityEntry>,
) -> Result<()> {
    sort_wallet_activity_entries(entries);
    let encoded = serde_json::to_vec(entries).context("Failed to encode Cashu wallet activity")?;
    localstore
        .kv_write(
            K_WALLET_ACTIVITY_PRIMARY_NAMESPACE,
            K_WALLET_ACTIVITY_SECONDARY_NAMESPACE,
            K_WALLET_ACTIVITY_KEY,
            &encoded,
        )
        .await
        .context("Failed to write Cashu wallet activity")?;
    Ok(())
}

async fn append_wallet_activity_entry(
    data_dir: &Path,
    entry: CashuWalletActivityEntry,
) -> Result<()> {
    let localstore = open_wallet_localstore(data_dir).await?;
    let mut entries = load_wallet_activity_entries_from_store(localstore.as_ref()).await?;
    entries.push(entry);
    save_wallet_activity_entries_to_store(localstore.as_ref(), &mut entries).await
}

async fn mark_wallet_activity_reclaimed(data_dir: &Path, operation_id: &str) -> Result<()> {
    let localstore = open_wallet_localstore(data_dir).await?;
    let mut entries = load_wallet_activity_entries_from_store(localstore.as_ref()).await?;
    let mut changed = false;
    for entry in &mut entries {
        if entry.kind == CashuWalletActivityKind::TokenSend
            && entry.operation_id.as_deref() == Some(operation_id)
            && entry.status != CashuWalletActivityStatus::Reclaimed
        {
            entry.status = CashuWalletActivityStatus::Reclaimed;
            changed = true;
        }
    }
    if changed {
        save_wallet_activity_entries_to_store(localstore.as_ref(), &mut entries).await?;
    }
    Ok(())
}

pub async fn load_wallet_activity(data_dir: &Path) -> Result<Vec<CashuWalletActivityEntry>> {
    let localstore = open_wallet_localstore(data_dir).await?;
    let mut entries = load_wallet_activity_entries_from_store(localstore.as_ref()).await?;
    if entries.is_empty() {
        return Ok(entries);
    }

    let repository = open_wallet_repository(data_dir).await?;
    let mut wallets_by_mint = HashMap::new();
    let mut mint_quotes_by_id = HashMap::new();

    for wallet in repository.get_wallets().await {
        for quote in wallet
            .localstore
            .get_mint_quotes()
            .await
            .context("Failed to load Cashu mint quotes for activity")?
        {
            mint_quotes_by_id.insert(quote.id.clone(), quote);
        }
        wallets_by_mint.insert(wallet.mint_url.to_string(), wallet);
    }

    let now_unix = wallet_activity_now_unix();
    let mut changed = false;
    for entry in &mut entries {
        match entry.kind {
            CashuWalletActivityKind::TopUp => {
                let next_status = if let Some(quote_id) = entry.quote_id.as_deref() {
                    match mint_quotes_by_id.get(quote_id) {
                        Some(quote)
                            if quote.state == MintQuoteState::Issued
                                || quote.state == MintQuoteState::Paid =>
                        {
                            CashuWalletActivityStatus::Complete
                        }
                        Some(quote)
                            if quote.state == MintQuoteState::Unpaid
                                && quote.expiry != 0
                                && quote.expiry < now_unix =>
                        {
                            CashuWalletActivityStatus::Expired
                        }
                        Some(_) => CashuWalletActivityStatus::Pending,
                        None if entry
                            .expires_at_unix
                            .is_some_and(|expiry| expiry != 0 && expiry < now_unix) =>
                        {
                            CashuWalletActivityStatus::Expired
                        }
                        None => entry.status.clone(),
                    }
                } else {
                    entry.status.clone()
                };

                if entry.status != next_status {
                    entry.status = next_status;
                    changed = true;
                }
            }
            CashuWalletActivityKind::TokenSend
                if entry.status == CashuWalletActivityStatus::Pending =>
            {
                let Some(operation_id) = entry.operation_id.as_deref() else {
                    continue;
                };
                let Some(wallet) = wallets_by_mint.get(&entry.mint_url) else {
                    continue;
                };
                let Ok(operation_uuid) = Uuid::parse_str(operation_id) else {
                    continue;
                };
                let saga = wallet
                    .localstore
                    .get_saga(&operation_uuid)
                    .await
                    .context("Failed to load Cashu send saga for activity")?;
                if saga.is_none() {
                    entry.status = CashuWalletActivityStatus::Complete;
                    changed = true;
                }
            }
            _ => {}
        }
    }

    if changed {
        save_wallet_activity_entries_to_store(localstore.as_ref(), &mut entries).await?;
    }

    Ok(entries)
}

pub fn normalize_mint_url(raw: &str) -> Result<String> {
    let mut url = Url::parse(raw).with_context(|| format!("Invalid mint URL: {raw}"))?;
    match url.scheme() {
        "http" | "https" => {}
        scheme => bail!("Unsupported mint URL scheme: {scheme}"),
    }
    if url.query().is_some() || url.fragment().is_some() {
        bail!("Mint URL must not include query or fragment");
    }

    let trimmed_path = url.path().trim_end_matches('/').to_string();
    if trimmed_path.is_empty() {
        url.set_path("");
    } else {
        url.set_path(&trimmed_path);
    }

    Ok(url.to_string().trim_end_matches('/').to_string())
}

pub fn load_or_create_wallet_seed(path: &Path) -> Result<[u8; 64]> {
    if path.exists() {
        let content = fs::read_to_string(path).context("Failed to read Cashu wallet seed")?;
        let seed_file: CashuWalletSeedFile =
            serde_json::from_str(&content).context("Failed to parse Cashu wallet seed")?;
        let seed_bytes = hex::decode(seed_file.seed_hex).context("Invalid Cashu wallet seed")?;
        let seed: [u8; 64] = seed_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("Cashu wallet seed must be 64 bytes"))?;
        return Ok(seed);
    }

    let mut seed = [0_u8; 64];
    rand::thread_rng().fill_bytes(&mut seed);
    write_wallet_seed(path, &seed)?;
    Ok(seed)
}

pub async fn open_wallet_repository(data_dir: &Path) -> Result<WalletRepository> {
    let seed = load_or_create_wallet_seed(&cashu_wallet_seed_path(data_dir))?;
    let localstore = open_wallet_localstore(data_dir).await?;
    let repository = WalletRepositoryBuilder::new()
        .localstore(localstore)
        .seed(seed)
        .build()
        .await
        .context("Failed to build Cashu wallet repository")?;
    Ok(repository)
}

pub async fn load_wallet_overview(
    data_dir: &Path,
    refresh_quotes: bool,
) -> Result<CashuWalletOverview> {
    let repository = open_wallet_repository(data_dir).await?;
    let mut warnings = Vec::new();

    if refresh_quotes {
        for wallet in repository.get_wallets().await {
            let mint_label = format!("{} ({})", wallet.mint_url, wallet.unit);
            if let Err(err) = wallet.recover_incomplete_sagas().await {
                warnings.push(format!(
                    "Failed to recover wallet state for {mint_label}: {err}"
                ));
                continue;
            }
            if let Err(err) = wallet.mint_unissued_quotes().await {
                warnings.push(format!(
                    "Failed to refresh pending mint quotes for {mint_label}: {err}"
                ));
            }
        }
    }

    let totals = repository
        .total_balance()
        .await
        .context("Failed to load Cashu wallet totals")?
        .into_iter()
        .map(|(unit, amount)| CashuUnitTotal {
            unit: unit.to_string(),
            balance: amount.to_u64(),
        })
        .collect();

    let entries = repository
        .get_balances()
        .await
        .context("Failed to load Cashu wallet balances")?
        .into_iter()
        .map(|(key, amount)| CashuWalletEntry {
            mint_url: key.mint_url.to_string(),
            unit: key.unit.to_string(),
            balance: amount.to_u64(),
        })
        .collect();

    Ok(CashuWalletOverview {
        totals,
        entries,
        warnings,
        legacy_state_detected: legacy_cashu_wallet_state_path(data_dir).exists(),
    })
}

pub async fn create_topup_quote(
    data_dir: &Path,
    mint_url: &str,
    amount_sat: u64,
) -> Result<CashuTopupQuote> {
    if amount_sat == 0 {
        bail!("Cashu topup amount must be greater than zero");
    }

    let normalized_mint = normalize_mint_url(mint_url)?;
    let mint_url =
        MintUrl::from_str(&normalized_mint).context("Failed to parse normalized mint URL")?;
    let repository = open_wallet_repository(data_dir).await?;
    let wallet = ensure_sat_wallet(&repository, &mint_url).await?;

    wallet
        .recover_incomplete_sagas()
        .await
        .context("Failed to recover Cashu wallet state before creating quote")?;
    wallet
        .mint_unissued_quotes()
        .await
        .context("Failed to refresh pending Cashu mint quotes before creating quote")?;

    let quote = wallet
        .mint_quote(
            PaymentMethod::BOLT11,
            Some(Amount::from(amount_sat)),
            None,
            None,
        )
        .await
        .context("Failed to create Cashu mint quote")?;

    let topup_quote = CashuTopupQuote {
        mint_url: normalized_mint,
        unit: CurrencyUnit::Sat.to_string(),
        amount: amount_sat,
        quote_id: quote.id,
        payment_request: quote.request,
        expiry_unix: quote.expiry,
    };

    append_wallet_activity_entry(
        data_dir,
        CashuWalletActivityEntry {
            id: wallet_activity_id(),
            kind: CashuWalletActivityKind::TopUp,
            status: CashuWalletActivityStatus::Pending,
            mint_url: topup_quote.mint_url.clone(),
            unit: topup_quote.unit.clone(),
            amount_sat,
            fee_sat: None,
            created_at_unix: wallet_activity_now_unix(),
            expires_at_unix: Some(topup_quote.expiry_unix),
            quote_id: Some(topup_quote.quote_id.clone()),
            operation_id: None,
            payment_request: Some(topup_quote.payment_request.clone()),
            token: None,
        },
    )
    .await
    .context("Failed to record Cashu top-up activity")?;

    Ok(topup_quote)
}

pub async fn load_mint_balance(data_dir: &Path, mint_url: &str) -> Result<CashuMintBalance> {
    let normalized_mint = normalize_mint_url(mint_url)?;
    let mint_url =
        MintUrl::from_str(&normalized_mint).context("Failed to parse normalized mint URL")?;
    let repository = open_wallet_repository(data_dir).await?;
    ensure_sat_wallet(&repository, &mint_url).await?;

    let balance_sat = repository
        .get_balances()
        .await
        .context("Failed to load Cashu wallet balances")?
        .into_iter()
        .find_map(|(key, amount)| {
            (key.mint_url == mint_url && key.unit == CurrencyUnit::Sat).then_some(amount.to_u64())
        })
        .unwrap_or_default();

    Ok(CashuMintBalance {
        mint_url: normalized_mint,
        unit: CurrencyUnit::Sat.to_string(),
        balance_sat,
    })
}

/// Move sat-denominated wallet balance between two caller-selected, approved mints.
///
/// `transfer_id` is an idempotency key: retries recover the stored CDK sagas and
/// resume the same destination mint quote and source melt quote. This function
/// deliberately performs no mint selection or trust inference.
pub async fn transfer_between_mints(
    data_dir: &Path,
    request: CashuCrossMintTransferRequest,
) -> Result<CashuCrossMintTransfer> {
    let _guard = CROSS_MINT_TRANSFER_LOCK.lock().await;
    let request = normalize_cross_mint_transfer_request(request)?;
    let source_mint = MintUrl::from_str(&request.source_mint_url)
        .context("Failed to parse normalized source mint URL")?;
    let destination_mint = MintUrl::from_str(&request.destination_mint_url)
        .context("Failed to parse normalized destination mint URL")?;
    let repository = open_wallet_repository(data_dir).await?;
    let source_wallet = ensure_sat_wallet(&repository, &source_mint).await?;
    let destination_wallet = ensure_sat_wallet(&repository, &destination_mint).await?;

    transfer_between_wallets(&source_wallet, &destination_wallet, request).await
}

fn normalize_cross_mint_transfer_request(
    mut request: CashuCrossMintTransferRequest,
) -> Result<CashuCrossMintTransferRequest> {
    request.transfer_id = request.transfer_id.trim().to_string();
    if request.transfer_id.is_empty()
        || request.transfer_id.len() > 128
        || !request
            .transfer_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("Cashu cross-mint transfer id must be 1-128 ASCII letters, digits, '-' or '_'");
    }
    if request.amount_sat == 0 {
        bail!("Cashu cross-mint transfer amount must be greater than zero");
    }
    request
        .amount_sat
        .checked_mul(1_000)
        .context("Cashu cross-mint transfer amount is too large")?;
    request.source_mint_url = normalize_mint_url(&request.source_mint_url)?;
    request.destination_mint_url = normalize_mint_url(&request.destination_mint_url)?;
    if request.source_mint_url == request.destination_mint_url {
        bail!("Cashu cross-mint transfer requires two different mints");
    }
    Ok(request)
}

async fn load_cross_mint_transfer_saga(
    wallet: &cdk::wallet::Wallet,
    transfer_id: &str,
) -> Result<Option<CashuCrossMintTransferSaga>> {
    let stored = wallet
        .localstore
        .kv_read(
            K_WALLET_ACTIVITY_PRIMARY_NAMESPACE,
            K_CROSS_MINT_TRANSFER_NAMESPACE,
            transfer_id,
        )
        .await
        .context("Failed to read Cashu cross-mint transfer saga")?;
    stored
        .map(|bytes| {
            serde_json::from_slice(&bytes).context("Failed to parse Cashu cross-mint transfer saga")
        })
        .transpose()
}

async fn save_cross_mint_transfer_saga(
    wallet: &cdk::wallet::Wallet,
    saga: &CashuCrossMintTransferSaga,
) -> Result<()> {
    let encoded =
        serde_json::to_vec(saga).context("Failed to encode Cashu cross-mint transfer saga")?;
    wallet
        .localstore
        .kv_write(
            K_WALLET_ACTIVITY_PRIMARY_NAMESPACE,
            K_CROSS_MINT_TRANSFER_NAMESPACE,
            &saga.request.transfer_id,
            &encoded,
        )
        .await
        .context("Failed to write Cashu cross-mint transfer saga")
}

fn validate_exact_bolt11_amount(payment_request: &str, amount_sat: u64) -> Result<()> {
    let invoice = Bolt11Invoice::from_str(payment_request)
        .context("Destination mint returned an invalid BOLT11 invoice")?;
    let expected_msat = amount_sat
        .checked_mul(1_000)
        .context("Cashu cross-mint transfer amount is too large")?;
    let invoice_msat = invoice
        .amount_milli_satoshis()
        .context("Destination mint returned an amountless BOLT11 invoice")?;
    if invoice_msat != expected_msat {
        bail!(
            "Destination mint invoice amount {invoice_msat} msat does not match requested amount {expected_msat} msat"
        );
    }
    Ok(())
}

async fn transfer_between_wallets(
    source_wallet: &cdk::wallet::Wallet,
    destination_wallet: &cdk::wallet::Wallet,
    request: CashuCrossMintTransferRequest,
) -> Result<CashuCrossMintTransfer> {
    let recovered_melts = source_wallet
        .finalize_pending_melts()
        .await
        .context("Failed to finalize pending source mint payments")?;
    source_wallet
        .recover_incomplete_sagas()
        .await
        .context("Failed to recover source mint wallet state")?;
    destination_wallet
        .recover_incomplete_sagas()
        .await
        .context("Failed to recover destination mint wallet state")?;

    let mut saga = match load_cross_mint_transfer_saga(source_wallet, &request.transfer_id).await? {
        Some(saga) => {
            if saga.version != K_CROSS_MINT_TRANSFER_VERSION || saga.request != request {
                bail!(
                    "Cashu cross-mint transfer id {} is already bound to a different request",
                    request.transfer_id
                );
            }
            if let Some(result) = saga.result {
                return Ok(result);
            }
            saga
        }
        None => {
            let saga = CashuCrossMintTransferSaga {
                version: K_CROSS_MINT_TRANSFER_VERSION,
                request: request.clone(),
                destination_balance_before_sat: destination_wallet
                    .total_balance()
                    .await
                    .context("Failed to load destination mint balance before transfer")?
                    .to_u64(),
                destination_mint_quote_id: None,
                destination_payment_request: None,
                source_melt_quote_id: None,
                melt_fee_reserve_sat: None,
                wallet_fee_sat: None,
                fee_paid_sat: None,
                result: None,
            };
            save_cross_mint_transfer_saga(source_wallet, &saga).await?;
            saga
        }
    };

    if saga.destination_mint_quote_id.is_none() {
        let quote = destination_wallet
            .mint_quote(
                PaymentMethod::BOLT11,
                Some(Amount::from(request.amount_sat)),
                None,
                None,
            )
            .await
            .context("Failed to create exact destination mint quote")?;
        if quote.amount != Some(Amount::from(request.amount_sat)) {
            bail!("Destination mint returned an incorrect mint quote amount");
        }
        validate_exact_bolt11_amount(&quote.request, request.amount_sat)?;
        saga.destination_mint_quote_id = Some(quote.id);
        saga.destination_payment_request = Some(quote.request);
        save_cross_mint_transfer_saga(source_wallet, &saga).await?;
    }

    let destination_quote_id = saga
        .destination_mint_quote_id
        .clone()
        .context("Cashu cross-mint saga has no destination quote id")?;
    let payment_request = saga
        .destination_payment_request
        .clone()
        .context("Cashu cross-mint saga has no destination invoice")?;
    validate_exact_bolt11_amount(&payment_request, request.amount_sat)?;

    if saga.source_melt_quote_id.is_none() {
        let quote = source_wallet
            .melt_quote(PaymentMethod::BOLT11, &payment_request, None, None)
            .await
            .context("Failed to preflight source mint melt quote")?;
        if quote.amount != Amount::from(request.amount_sat) {
            bail!("Source mint returned an incorrect melt quote amount");
        }
        let fee_reserve_sat = quote.fee_reserve.to_u64();
        if fee_reserve_sat > request.max_fee_sat {
            bail!(
                "Source mint melt fee reserve {fee_reserve_sat} sat exceeds caller maximum {} sat",
                request.max_fee_sat
            );
        }
        saga.source_melt_quote_id = Some(quote.id);
        saga.melt_fee_reserve_sat = Some(fee_reserve_sat);
        save_cross_mint_transfer_saga(source_wallet, &saga).await?;
    }

    let source_quote_id = saga
        .source_melt_quote_id
        .clone()
        .context("Cashu cross-mint saga has no source quote id")?;
    let source_quote = source_wallet
        .localstore
        .get_melt_quote(&source_quote_id)
        .await
        .context("Failed to load source mint melt quote")?
        .context("Stored source mint melt quote is missing")?;
    if source_quote.request != payment_request
        || source_quote.amount != Amount::from(request.amount_sat)
        || source_quote.fee_reserve.to_u64()
            != saga
                .melt_fee_reserve_sat
                .context("Cashu cross-mint saga has no melt fee reserve")?
    {
        bail!("Stored source mint melt quote does not match the cross-mint transfer");
    }

    let mut paid_transaction = source_wallet
        .list_transactions(Some(cdk::wallet::types::TransactionDirection::Outgoing))
        .await
        .context("Failed to load source mint transactions")?
        .into_iter()
        .find(|transaction| transaction.quote_id.as_deref() == Some(source_quote_id.as_str()));

    if paid_transaction.is_none() {
        if let Some(finalized) = recovered_melts
            .into_iter()
            .find(|melt| melt.quote_id() == source_quote_id)
        {
            if finalized.state() != MeltQuoteState::Paid {
                bail!(
                    "Recovered source mint payment finished in unexpected state {}",
                    finalized.state()
                );
            }
            saga.fee_paid_sat = Some(finalized.fee_paid().to_u64());
        }

        match source_quote.state {
            MeltQuoteState::Unpaid => {
                let prepared = source_wallet
                    .prepare_melt(&source_quote_id, HashMap::new())
                    .await
                    .context("Failed to prepare source mint payment")?;
                let wallet_fee_sat = prepared.total_fee_with_swap().to_u64();
                let preflight_fee_sat = source_quote
                    .fee_reserve
                    .to_u64()
                    .checked_add(wallet_fee_sat)
                    .context("Cashu cross-mint fee overflow")?;
                if preflight_fee_sat > request.max_fee_sat {
                    prepared
                        .cancel()
                        .await
                        .context("Failed to cancel over-limit source mint payment")?;
                    bail!(
                        "Source mint preflight fee {preflight_fee_sat} sat exceeds caller maximum {} sat",
                        request.max_fee_sat
                    );
                }
                saga.wallet_fee_sat = Some(wallet_fee_sat);
                save_cross_mint_transfer_saga(source_wallet, &saga).await?;
                let finalized = prepared
                    .confirm()
                    .await
                    .context("Failed to execute source mint payment")?;
                if finalized.state() != MeltQuoteState::Paid {
                    bail!(
                        "Source mint payment finished in unexpected state {}",
                        finalized.state()
                    );
                }
                saga.fee_paid_sat = Some(finalized.fee_paid().to_u64());
                save_cross_mint_transfer_saga(source_wallet, &saga).await?;
            }
            MeltQuoteState::Paid => {}
            state => bail!("Source mint payment is not recoverable from state {state}"),
        }

        paid_transaction = source_wallet
            .list_transactions(Some(cdk::wallet::types::TransactionDirection::Outgoing))
            .await
            .context("Failed to reload source mint transactions")?
            .into_iter()
            .find(|transaction| transaction.quote_id.as_deref() == Some(source_quote_id.as_str()));
    }

    let paid_transaction =
        paid_transaction.context("Paid source mint transfer has no durable wallet transaction")?;
    let fee_paid_sat = paid_transaction.fee.to_u64();
    saga.fee_paid_sat = Some(fee_paid_sat);
    save_cross_mint_transfer_saga(source_wallet, &saga).await?;

    let destination_quote = destination_wallet
        .check_mint_quote_status(&destination_quote_id)
        .await
        .context("Failed to refresh destination mint quote")?;
    if destination_quote.amount != Some(Amount::from(request.amount_sat)) {
        bail!("Destination mint quote amount changed during transfer");
    }
    if destination_quote.state == MintQuoteState::Paid {
        destination_wallet
            .mint(
                &destination_quote_id,
                cdk::amount::SplitTarget::default(),
                None,
            )
            .await
            .context("Failed to issue paid destination mint quote")?;
    } else if destination_quote.state != MintQuoteState::Issued {
        bail!(
            "Paid source transfer has destination mint quote in unexpected state {}",
            destination_quote.state
        );
    }

    let destination_balance_after_sat = destination_wallet
        .total_balance()
        .await
        .context("Failed to load destination mint balance after transfer")?
        .to_u64();
    let expected_balance = saga
        .destination_balance_before_sat
        .checked_add(request.amount_sat)
        .context("Destination mint balance overflow")?;
    if destination_balance_after_sat != expected_balance {
        bail!(
            "Destination mint balance is {destination_balance_after_sat} sat; expected {expected_balance} sat after transfer"
        );
    }
    if fee_paid_sat > request.max_fee_sat {
        bail!(
            "Source mint charged {fee_paid_sat} sat after a maximum {} sat preflight",
            request.max_fee_sat
        );
    }

    let result = CashuCrossMintTransfer {
        transfer_id: request.transfer_id.clone(),
        source_mint_url: request.source_mint_url.clone(),
        destination_mint_url: request.destination_mint_url.clone(),
        unit: CurrencyUnit::Sat.to_string(),
        amount_sat: request.amount_sat,
        max_fee_sat: request.max_fee_sat,
        melt_fee_reserve_sat: saga.melt_fee_reserve_sat.unwrap_or_default(),
        wallet_fee_sat: saga.wallet_fee_sat.unwrap_or_default(),
        fee_paid_sat,
        source_melt_quote_id: source_quote_id,
        destination_mint_quote_id: destination_quote_id,
        destination_balance_before_sat: saga.destination_balance_before_sat,
        destination_balance_after_sat,
    };
    saga.result = Some(result.clone());
    save_cross_mint_transfer_saga(source_wallet, &saga).await?;
    Ok(result)
}

pub async fn send_payment_token(
    data_dir: &Path,
    mint_url: &str,
    amount_sat: u64,
) -> Result<CashuSentPayment> {
    if amount_sat == 0 {
        bail!("Cashu payment amount must be greater than zero");
    }

    let normalized_mint = normalize_mint_url(mint_url)?;
    let mint_url =
        MintUrl::from_str(&normalized_mint).context("Failed to parse normalized mint URL")?;
    let repository = open_wallet_repository(data_dir).await?;
    let wallet = ensure_sat_wallet(&repository, &mint_url).await?;

    wallet
        .recover_incomplete_sagas()
        .await
        .context("Failed to recover Cashu wallet state before sending payment")?;

    let prepared = wallet
        .prepare_send(
            Amount::from(amount_sat),
            SendOptions {
                include_fee: true,
                ..Default::default()
            },
        )
        .await
        .context("Failed to prepare Cashu payment token")?;
    let operation_id = prepared.operation_id().to_string();
    let send_fee_sat = prepared.send_fee().to_u64();
    let token = prepared
        .confirm(None)
        .await
        .context("Failed to create Cashu payment token")?;

    let payment = CashuSentPayment {
        mint_url: normalized_mint,
        unit: CurrencyUnit::Sat.to_string(),
        amount_sat,
        send_fee_sat,
        operation_id,
        token: token.to_string(),
    };

    append_wallet_activity_entry(
        data_dir,
        CashuWalletActivityEntry {
            id: wallet_activity_id(),
            kind: CashuWalletActivityKind::TokenSend,
            status: CashuWalletActivityStatus::Pending,
            mint_url: payment.mint_url.clone(),
            unit: payment.unit.clone(),
            amount_sat: payment.amount_sat,
            fee_sat: Some(payment.send_fee_sat),
            created_at_unix: wallet_activity_now_unix(),
            expires_at_unix: None,
            quote_id: None,
            operation_id: Some(payment.operation_id.clone()),
            payment_request: None,
            token: Some(payment.token.clone()),
        },
    )
    .await
    .context("Failed to record Cashu token activity")?;

    Ok(payment)
}

pub async fn send_lightning_payment(
    data_dir: &Path,
    mint_url: &str,
    payment_request: &str,
) -> Result<CashuLightningPayment> {
    let normalized_mint = normalize_mint_url(mint_url)?;
    let mint_url =
        MintUrl::from_str(&normalized_mint).context("Failed to parse normalized mint URL")?;
    let repository = open_wallet_repository(data_dir).await?;
    let wallet = ensure_sat_wallet(&repository, &mint_url).await?;

    let mut payment = send_lightning_payment_with_wallet(&wallet, payment_request).await?;
    payment.mint_url = normalized_mint;
    append_wallet_activity_entry(
        data_dir,
        CashuWalletActivityEntry {
            id: wallet_activity_id(),
            kind: CashuWalletActivityKind::LightningPayment,
            status: CashuWalletActivityStatus::Complete,
            mint_url: payment.mint_url.clone(),
            unit: payment.unit.clone(),
            amount_sat: payment.amount_sat,
            fee_sat: Some(payment.fee_paid_sat),
            created_at_unix: wallet_activity_now_unix(),
            expires_at_unix: None,
            quote_id: Some(payment.quote_id.clone()),
            operation_id: None,
            payment_request: Some(payment_request.to_string()),
            token: None,
        },
    )
    .await
    .context("Failed to record Cashu Lightning payment activity")?;
    Ok(payment)
}

pub async fn receive_payment_token(
    data_dir: &Path,
    encoded_token: &str,
) -> Result<CashuReceivedPayment> {
    let token = Token::from_str(encoded_token).context("Failed to parse Cashu token")?;
    let mint_url = token
        .mint_url()
        .context("Cashu token must contain exactly one mint")?;
    let unit = token.unit().unwrap_or_default();
    if unit != CurrencyUnit::Sat {
        bail!("Unsupported Cashu token unit: {unit}");
    }
    let normalized_mint = normalize_mint_url(&mint_url.to_string())?;

    let repository = open_wallet_repository(data_dir).await?;
    let wallet = ensure_sat_wallet(&repository, &mint_url).await?;
    wallet
        .recover_incomplete_sagas()
        .await
        .context("Failed to recover Cashu wallet state before receiving payment")?;

    let amount_received = wallet
        .receive(encoded_token, ReceiveOptions::default())
        .await
        .context("Failed to receive Cashu payment token")?;

    let payment = CashuReceivedPayment {
        mint_url: normalized_mint,
        unit: CurrencyUnit::Sat.to_string(),
        amount_sat: amount_received.to_u64(),
    };

    append_wallet_activity_entry(
        data_dir,
        CashuWalletActivityEntry {
            id: wallet_activity_id(),
            kind: CashuWalletActivityKind::TokenReceive,
            status: CashuWalletActivityStatus::Complete,
            mint_url: payment.mint_url.clone(),
            unit: payment.unit.clone(),
            amount_sat: payment.amount_sat,
            fee_sat: None,
            created_at_unix: wallet_activity_now_unix(),
            expires_at_unix: None,
            quote_id: None,
            operation_id: None,
            payment_request: None,
            token: None,
        },
    )
    .await
    .context("Failed to record Cashu receive activity")?;

    Ok(payment)
}

pub async fn import_payment_proofs(
    data_dir: &Path,
    mint_url: &str,
    unit: &str,
    proofs_json: &str,
) -> Result<CashuReceivedPayment> {
    let unit = unit.trim();
    if unit != CurrencyUnit::Sat.to_string() {
        bail!("Unsupported Cashu proof unit: {unit}");
    }

    let normalized_mint = normalize_mint_url(mint_url)?;
    let mint_url =
        MintUrl::from_str(&normalized_mint).context("Failed to parse normalized mint URL")?;
    let proofs: Vec<Proof> =
        serde_json::from_str(proofs_json).context("Failed to parse Cashu payment proofs")?;

    let repository = open_wallet_repository(data_dir).await?;
    let wallet = ensure_sat_wallet(&repository, &mint_url).await?;
    wallet
        .recover_incomplete_sagas()
        .await
        .context("Failed to recover Cashu wallet state before importing proofs")?;

    let mut proof_infos = Vec::with_capacity(proofs.len());
    for proof in proofs {
        proof_infos.push(
            ProofInfo::new(proof, mint_url.clone(), State::Unspent, CurrencyUnit::Sat)
                .context("Failed to prepare Cashu proof import")?,
        );
    }

    let existing_ys = wallet
        .localstore
        .get_proofs_by_ys(proof_infos.iter().map(|proof| proof.y).collect())
        .await
        .context("Failed to check existing Cashu proofs")?
        .into_iter()
        .map(|proof| proof.y.to_string())
        .collect::<HashSet<_>>();

    let mut imported_amount_sat = 0_u64;
    let proof_infos = proof_infos
        .into_iter()
        .filter(|proof| !existing_ys.contains(&proof.y.to_string()))
        .inspect(|proof| {
            imported_amount_sat = imported_amount_sat.saturating_add(proof.proof.amount.to_u64());
        })
        .collect::<Vec<_>>();

    if !proof_infos.is_empty() {
        wallet
            .localstore
            .update_proofs(proof_infos, vec![])
            .await
            .context("Failed to import Cashu proofs into wallet")?;

        append_wallet_activity_entry(
            data_dir,
            CashuWalletActivityEntry {
                id: wallet_activity_id(),
                kind: CashuWalletActivityKind::ChannelCollect,
                status: CashuWalletActivityStatus::Complete,
                mint_url: normalized_mint.clone(),
                unit: CurrencyUnit::Sat.to_string(),
                amount_sat: imported_amount_sat,
                fee_sat: None,
                created_at_unix: wallet_activity_now_unix(),
                expires_at_unix: None,
                quote_id: None,
                operation_id: None,
                payment_request: None,
                token: None,
            },
        )
        .await
        .context("Failed to record Cashu channel collection activity")?;
    }

    Ok(CashuReceivedPayment {
        mint_url: normalized_mint,
        unit: CurrencyUnit::Sat.to_string(),
        amount_sat: imported_amount_sat,
    })
}

pub async fn revoke_pending_payment(
    data_dir: &Path,
    mint_url: &str,
    operation_id: &str,
) -> Result<u64> {
    let normalized_mint = normalize_mint_url(mint_url)?;
    let mint_url =
        MintUrl::from_str(&normalized_mint).context("Failed to parse normalized mint URL")?;
    let repository = open_wallet_repository(data_dir).await?;
    let wallet = ensure_sat_wallet(&repository, &mint_url).await?;
    wallet
        .recover_incomplete_sagas()
        .await
        .context("Failed to recover Cashu wallet state before revoking payment")?;

    let normalized_operation_id = operation_id.to_string();
    let operation_id = normalized_operation_id
        .parse()
        .context("Invalid Cashu send operation id")?;
    let amount = wallet
        .revoke_send(operation_id)
        .await
        .context("Failed to revoke Cashu payment token")?;
    mark_wallet_activity_reclaimed(data_dir, &normalized_operation_id)
        .await
        .context("Failed to record reclaimed Cashu token")?;
    Ok(amount.to_u64())
}

fn write_wallet_seed(path: &Path, seed: &[u8; 64]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Failed to create Cashu wallet directory")?;
    }

    let seed_file = CashuWalletSeedFile {
        version: CASHU_WALLET_SEED_VERSION,
        seed_hex: hex::encode(seed),
    };
    let content =
        serde_json::to_string_pretty(&seed_file).context("Failed to encode Cashu wallet seed")?;
    fs::write(path, content).context("Failed to write Cashu wallet seed")?;
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .context("Failed to secure Cashu wallet seed permissions")?;
    }
    Ok(())
}

async fn ensure_sat_wallet(
    repository: &WalletRepository,
    mint_url: &MintUrl,
) -> Result<cdk::wallet::Wallet> {
    let wallet = if repository.has_wallet(mint_url, &CurrencyUnit::Sat).await {
        repository
            .get_wallet(mint_url, &CurrencyUnit::Sat)
            .await
            .context("Failed to load existing Cashu sat wallet")?
    } else {
        repository
            .create_wallet(mint_url.clone(), CurrencyUnit::Sat, None)
            .await
            .context("Failed to create Cashu sat wallet")?
    };

    wallet
        .localstore
        .add_mint(mint_url.clone(), None)
        .await
        .context("Failed to persist Cashu mint metadata")?;

    Ok(wallet)
}

async fn send_lightning_payment_with_wallet(
    wallet: &cdk::wallet::Wallet,
    payment_request: &str,
) -> Result<CashuLightningPayment> {
    wallet
        .recover_incomplete_sagas()
        .await
        .context("Failed to recover Cashu wallet state before sending Lightning payment")?;

    let quote = wallet
        .melt_quote(PaymentMethod::BOLT11, payment_request, None, None)
        .await
        .context("Failed to create Cashu Lightning melt quote")?;
    let prepared = wallet
        .prepare_melt(&quote.id, HashMap::new())
        .await
        .context("Failed to prepare Cashu Lightning payment")?;
    let finalized = prepared
        .confirm()
        .await
        .context("Failed to execute Cashu Lightning payment")?;

    if finalized.state() != MeltQuoteState::Paid {
        bail!(
            "Cashu Lightning payment finished in unexpected state {}",
            finalized.state()
        );
    }

    let preimage = finalized
        .payment_proof()
        .filter(|proof| !proof.is_empty())
        .map(str::to_owned)
        .context("Cashu Lightning payment completed without a preimage")?;

    Ok(CashuLightningPayment {
        mint_url: wallet.mint_url.to_string(),
        unit: wallet.unit.to_string(),
        amount_sat: finalized.amount().to_u64(),
        fee_paid_sat: finalized.fee_paid().to_u64(),
        quote_id: finalized.quote_id().to_string(),
        preimage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cdk::cdk_database::WalletDatabase;
    use cdk::nuts::{
        BatchCheckMintQuoteRequest, BatchMintRequest, BlindSignature, CheckStateRequest,
        CheckStateResponse, CurrencyUnit, Id, KeySet, KeySetInfo, Keys, KeysetResponse,
        MeltQuoteBolt11Response, MeltRequest, MintInfo, MintQuoteBolt11Response, MintRequest,
        MintResponse, PaymentMethod, Proof, RestoreRequest, RestoreResponse, SecretKey, State,
        SwapRequest, SwapResponse,
    };
    use cdk::secret::Secret;
    use cdk::wallet::{types::ProofInfo, MintConnector, WalletBuilder};
    use cdk::{Amount, Error};
    use cdk_common::{MeltQuoteRequest, MeltQuoteResponse, MintQuoteRequest, MintQuoteResponse};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::Arc;

    const K_VALID_BOLT11_INVOICE: &str = "lnbc2500u1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygspp5qqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqdq5xysxxatsyp3k7enxv4jsxqzpu9qrsgquk0rl77nj30yxdy8j9vdx85fkpmdla2087ne0xh8nhedh8w27kyke0lp53ut353s06fv3qfegext0eh0ymjpf39tuven09sam30g4vgpfna3rh";
    const K_TEST_EXPIRY_UNIX: u64 = 4_102_444_800;

    #[derive(Debug, Default)]
    struct CrossMintTestState {
        paid: AtomicBool,
        destination_amount_sat: AtomicU64,
        delay_destination_once: AtomicBool,
        mint_quote_calls: AtomicUsize,
        melt_calls: AtomicUsize,
        mint_calls: AtomicUsize,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestMintRole {
        Source,
        Destination,
    }

    #[derive(Debug, Clone)]
    struct LightningMockMintConnector {
        keyset: KeySet,
        quote_id: String,
        preimage: String,
        role: TestMintRole,
        fee_reserve_sat: u64,
        state: Arc<CrossMintTestState>,
    }

    impl LightningMockMintConnector {
        fn new(keyset: KeySet, quote_id: &str, preimage: &str) -> Self {
            Self {
                keyset,
                quote_id: quote_id.to_string(),
                preimage: preimage.to_string(),
                role: TestMintRole::Source,
                fee_reserve_sat: 0,
                state: Arc::new(CrossMintTestState::default()),
            }
        }

        fn cross_mint_source(
            keyset: KeySet,
            state: Arc<CrossMintTestState>,
            fee_reserve_sat: u64,
        ) -> Self {
            Self {
                keyset,
                quote_id: "source-melt-quote".to_string(),
                preimage: "00ff".to_string(),
                role: TestMintRole::Source,
                fee_reserve_sat,
                state,
            }
        }

        fn cross_mint_destination(keyset: KeySet, state: Arc<CrossMintTestState>) -> Self {
            Self {
                keyset,
                quote_id: "destination-mint-quote".to_string(),
                preimage: String::new(),
                role: TestMintRole::Destination,
                fee_reserve_sat: 0,
                state,
            }
        }

        fn keyset_info(&self) -> KeySetInfo {
            KeySetInfo {
                id: self.keyset.id,
                unit: self.keyset.unit.clone(),
                active: self.keyset.active.unwrap_or(true),
                input_fee_ppk: self.keyset.input_fee_ppk,
                final_expiry: self.keyset.final_expiry,
            }
        }
    }

    #[async_trait]
    impl MintConnector for LightningMockMintConnector {
        async fn fetch_lnurl_pay_request(
            &self,
            _url: &str,
        ) -> Result<cdk::wallet::LnurlPayResponse, Error> {
            unreachable!("unused in Cashu Lightning payment test")
        }

        async fn fetch_lnurl_invoice(
            &self,
            _url: &str,
        ) -> Result<cdk::wallet::LnurlPayInvoiceResponse, Error> {
            unreachable!("unused in Cashu Lightning payment test")
        }

        async fn get_mint_keys(&self) -> Result<Vec<KeySet>, Error> {
            Ok(vec![self.keyset.clone()])
        }

        async fn get_mint_keyset(&self, keyset_id: Id) -> Result<KeySet, Error> {
            if keyset_id == self.keyset.id {
                Ok(self.keyset.clone())
            } else {
                Err(Error::UnknownKeySet)
            }
        }

        async fn get_mint_keysets(&self) -> Result<KeysetResponse, Error> {
            Ok(KeysetResponse {
                keysets: vec![self.keyset_info()],
            })
        }

        async fn post_mint_quote(
            &self,
            request: MintQuoteRequest,
        ) -> Result<MintQuoteResponse<String>, Error> {
            if self.role != TestMintRole::Destination {
                unreachable!("unused in Cashu Lightning payment test")
            }
            let MintQuoteRequest::Bolt11(request) = request else {
                unreachable!("unused payment method in cross-mint test")
            };
            self.state
                .destination_amount_sat
                .store(request.amount.to_u64(), Ordering::SeqCst);
            self.state.mint_quote_calls.fetch_add(1, Ordering::SeqCst);
            Ok(MintQuoteResponse::Bolt11(MintQuoteBolt11Response {
                quote: self.quote_id.clone(),
                request: K_VALID_BOLT11_INVOICE.to_string(),
                amount: Some(request.amount),
                unit: Some(CurrencyUnit::Sat),
                state: MintQuoteState::Unpaid,
                expiry: Some(K_TEST_EXPIRY_UNIX),
                pubkey: request.pubkey,
            }))
        }

        async fn get_mint_quote_status(
            &self,
            _method: PaymentMethod,
            quote_id: &str,
        ) -> Result<MintQuoteResponse<String>, Error> {
            if self.role != TestMintRole::Destination || quote_id != self.quote_id {
                unreachable!("unused in Cashu Lightning payment test")
            }
            let paid = self.state.paid.load(Ordering::SeqCst);
            let delayed = paid
                && self
                    .state
                    .delay_destination_once
                    .swap(false, Ordering::SeqCst);
            Ok(MintQuoteResponse::Bolt11(MintQuoteBolt11Response {
                quote: self.quote_id.clone(),
                request: K_VALID_BOLT11_INVOICE.to_string(),
                amount: Some(Amount::from(
                    self.state.destination_amount_sat.load(Ordering::SeqCst),
                )),
                unit: Some(CurrencyUnit::Sat),
                state: if paid && !delayed {
                    MintQuoteState::Paid
                } else {
                    MintQuoteState::Unpaid
                },
                expiry: Some(K_TEST_EXPIRY_UNIX),
                pubkey: None,
            }))
        }

        async fn post_mint(
            &self,
            _method: &PaymentMethod,
            request: MintRequest<String>,
        ) -> Result<MintResponse, Error> {
            if self.role != TestMintRole::Destination || request.quote != self.quote_id {
                unreachable!("unused in Cashu Lightning payment test")
            }
            self.state.mint_calls.fetch_add(1, Ordering::SeqCst);
            Ok(MintResponse {
                signatures: request
                    .outputs
                    .into_iter()
                    .map(|output| BlindSignature {
                        amount: output.amount,
                        keyset_id: output.keyset_id,
                        c: SecretKey::generate().public_key(),
                        dleq: None,
                    })
                    .collect(),
            })
        }

        async fn post_melt_quote(
            &self,
            request: MeltQuoteRequest,
        ) -> Result<MeltQuoteResponse<String>, Error> {
            let MeltQuoteRequest::Bolt11(request) = request else {
                unreachable!("unused payment method in Cashu Lightning payment test")
            };
            let amount_msat = request
                .request
                .amount_milli_satoshis()
                .ok_or(Error::InvoiceAmountUndefined)?;
            Ok(MeltQuoteResponse::Bolt11(MeltQuoteBolt11Response {
                quote: self.quote_id.clone(),
                amount: Amount::from(amount_msat / 1000),
                fee_reserve: Amount::from(self.fee_reserve_sat),
                state: MeltQuoteState::Unpaid,
                expiry: K_TEST_EXPIRY_UNIX,
                payment_preimage: None,
                change: None,
                request: Some(request.request.to_string()),
                unit: Some(CurrencyUnit::Sat),
            }))
        }

        async fn get_melt_quote_status(
            &self,
            _method: PaymentMethod,
            quote_id: &str,
        ) -> Result<MeltQuoteResponse<String>, Error> {
            if self.role != TestMintRole::Source || quote_id != self.quote_id {
                unreachable!("unused in Cashu Lightning payment test")
            }
            Ok(MeltQuoteResponse::Bolt11(MeltQuoteBolt11Response {
                quote: self.quote_id.clone(),
                amount: Amount::from(
                    Bolt11Invoice::from_str(K_VALID_BOLT11_INVOICE)
                        .unwrap()
                        .amount_milli_satoshis()
                        .unwrap()
                        / 1_000,
                ),
                fee_reserve: Amount::from(self.fee_reserve_sat),
                state: if self.state.paid.load(Ordering::SeqCst) {
                    MeltQuoteState::Paid
                } else {
                    MeltQuoteState::Unpaid
                },
                expiry: K_TEST_EXPIRY_UNIX,
                payment_preimage: self
                    .state
                    .paid
                    .load(Ordering::SeqCst)
                    .then(|| self.preimage.clone()),
                change: None,
                request: None,
                unit: Some(CurrencyUnit::Sat),
            }))
        }

        async fn post_melt(
            &self,
            _method: &PaymentMethod,
            request: MeltRequest<String>,
        ) -> Result<MeltQuoteBolt11Response<String>, Error> {
            if request.quote_id() != &self.quote_id {
                return Err(Error::Custom("unexpected quote id".to_string()));
            }
            self.state.melt_calls.fetch_add(1, Ordering::SeqCst);
            self.state.paid.store(true, Ordering::SeqCst);
            Ok(MeltQuoteBolt11Response {
                quote: self.quote_id.clone(),
                amount: Amount::from(250_000_u64),
                fee_reserve: Amount::from(self.fee_reserve_sat),
                state: MeltQuoteState::Paid,
                expiry: K_TEST_EXPIRY_UNIX,
                payment_preimage: Some(self.preimage.clone()),
                change: None,
                request: None,
                unit: Some(CurrencyUnit::Sat),
            })
        }

        async fn post_swap(&self, _request: SwapRequest) -> Result<SwapResponse, Error> {
            unreachable!("exact proofs avoid the swap path in this test")
        }

        async fn get_mint_info(&self) -> Result<MintInfo, Error> {
            Ok(MintInfo::new())
        }

        async fn post_check_state(
            &self,
            _request: CheckStateRequest,
        ) -> Result<CheckStateResponse, Error> {
            unreachable!("unused in Cashu Lightning payment test")
        }

        async fn post_restore(&self, _request: RestoreRequest) -> Result<RestoreResponse, Error> {
            unreachable!("unused in Cashu Lightning payment test")
        }

        async fn get_auth_wallet(&self) -> Option<cdk::wallet::AuthWallet> {
            None
        }

        async fn set_auth_wallet(&self, _wallet: Option<cdk::wallet::AuthWallet>) {}

        async fn post_batch_check_mint_quote_status(
            &self,
            _method: &PaymentMethod,
            _request: BatchCheckMintQuoteRequest<String>,
        ) -> Result<Vec<MintQuoteBolt11Response<String>>, Error> {
            unreachable!("unused in Cashu Lightning payment test")
        }

        async fn post_batch_mint(
            &self,
            _method: &PaymentMethod,
            _request: BatchMintRequest<String>,
        ) -> Result<MintResponse, Error> {
            unreachable!("unused in Cashu Lightning payment test")
        }
    }

    fn build_test_keyset(max_amount_sat: u64) -> KeySet {
        let mut keys_map = BTreeMap::new();
        let mut current = 1_u64;
        let mut seed_byte = 1_u8;
        while current <= max_amount_sat {
            let secret_key = SecretKey::from_slice(&[seed_byte; 32]).unwrap();
            keys_map.insert(Amount::from(current), secret_key.public_key());
            current <<= 1;
            seed_byte = seed_byte.saturating_add(1);
        }

        let keys = Keys::new(keys_map);
        KeySet {
            id: Id::v1_from_keys(&keys),
            unit: CurrencyUnit::Sat,
            active: Some(true),
            keys,
            input_fee_ppk: 0,
            final_expiry: None,
        }
    }

    fn make_test_proof(keyset_id: Id, amount: u64) -> Proof {
        Proof::new(
            Amount::from(amount),
            keyset_id,
            Secret::generate(),
            SecretKey::generate().public_key(),
        )
    }

    fn make_proof_info(keyset_id: Id, amount: u64, mint_url: MintUrl) -> ProofInfo {
        let proof = make_test_proof(keyset_id, amount);
        ProofInfo::new(proof, mint_url, State::Unspent, CurrencyUnit::Sat).unwrap()
    }

    fn binary_proof_infos(mint_url: MintUrl, keyset_id: Id, amount_sat: u64) -> Vec<ProofInfo> {
        let mut proofs = Vec::new();
        let mut remaining = amount_sat;
        let mut bit = 1_u64 << (63 - remaining.leading_zeros() as u64);
        while bit > 0 {
            if remaining >= bit {
                proofs.push(make_proof_info(keyset_id, bit, mint_url.clone()));
                remaining -= bit;
            }
            bit >>= 1;
        }
        proofs
    }

    async fn cross_mint_test_wallets(
        fee_reserve_sat: u64,
        state: Arc<CrossMintTestState>,
    ) -> (cdk::wallet::Wallet, cdk::wallet::Wallet) {
        let keyset = build_test_keyset(250_002);
        let source_mint: MintUrl = "https://source.example".parse().unwrap();
        let destination_mint: MintUrl = "https://destination.example".parse().unwrap();
        let db = cdk_sqlite::wallet::memory::empty().await.unwrap();
        db.update_proofs(
            binary_proof_infos(source_mint.clone(), keyset.id, 250_002),
            vec![],
        )
        .await
        .unwrap();
        let db = Arc::new(db);
        let source = WalletBuilder::new()
            .mint_url(source_mint)
            .unit(CurrencyUnit::Sat)
            .localstore(db.clone())
            .seed([11_u8; 64])
            .shared_client(Arc::new(LightningMockMintConnector::cross_mint_source(
                keyset.clone(),
                state.clone(),
                fee_reserve_sat,
            )))
            .build()
            .unwrap();
        let destination = WalletBuilder::new()
            .mint_url(destination_mint)
            .unit(CurrencyUnit::Sat)
            .localstore(db)
            .seed([12_u8; 64])
            .shared_client(Arc::new(
                LightningMockMintConnector::cross_mint_destination(keyset, state),
            ))
            .build()
            .unwrap();
        (source, destination)
    }

    fn cross_mint_request(transfer_id: &str) -> CashuCrossMintTransferRequest {
        CashuCrossMintTransferRequest {
            transfer_id: transfer_id.to_string(),
            source_mint_url: "https://source.example".to_string(),
            destination_mint_url: "https://destination.example".to_string(),
            amount_sat: 250_000,
            max_fee_sat: 2,
        }
    }

    #[test]
    fn test_normalize_mint_url_trims_trailing_slash_and_rejects_query() {
        assert_eq!(
            normalize_mint_url("https://mint.example/").unwrap(),
            "https://mint.example"
        );
        assert_eq!(
            normalize_mint_url("http://127.0.0.1:3338/api/v1/").unwrap(),
            "http://127.0.0.1:3338/api/v1"
        );
        assert!(normalize_mint_url("wss://mint.example").is_err());
        assert!(normalize_mint_url("https://mint.example/?x=1").is_err());
    }

    #[test]
    fn test_cashu_wallet_seed_roundtrip_and_paths() {
        let temp_dir = tempfile::tempdir().unwrap();
        let seed_path = cashu_wallet_seed_path(temp_dir.path());
        let db_path = cashu_wallet_db_path(temp_dir.path());
        assert_eq!(seed_path, temp_dir.path().join("cashu").join("seed.json"));
        assert_eq!(db_path, temp_dir.path().join("cashu").join("wallet.sqlite"));

        let seed = load_or_create_wallet_seed(&seed_path).unwrap();
        assert_eq!(seed.len(), 64);
        let restored = load_or_create_wallet_seed(&seed_path).unwrap();
        assert_eq!(restored, seed);
    }

    #[tokio::test]
    async fn test_wallet_overview_loads_stored_wallets() {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo = open_wallet_repository(temp_dir.path()).await.unwrap();
        let mint_url: MintUrl = "https://mint.example".parse().unwrap();
        ensure_sat_wallet(&repo, &mint_url).await.unwrap();

        let overview = load_wallet_overview(temp_dir.path(), false).await.unwrap();
        assert_eq!(
            overview.entries,
            vec![CashuWalletEntry {
                mint_url: "https://mint.example".to_string(),
                unit: "sat".to_string(),
                balance: 0,
            }]
        );
        assert_eq!(
            overview.totals,
            vec![CashuUnitTotal {
                unit: "sat".to_string(),
                balance: 0,
            }]
        );
    }

    #[tokio::test]
    async fn test_load_mint_balance_returns_zero_for_known_wallet() {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo = open_wallet_repository(temp_dir.path()).await.unwrap();
        let mint_url: MintUrl = "https://mint.example".parse().unwrap();
        ensure_sat_wallet(&repo, &mint_url).await.unwrap();

        let balance = load_mint_balance(temp_dir.path(), "https://mint.example")
            .await
            .unwrap();
        assert_eq!(balance.mint_url, "https://mint.example");
        assert_eq!(balance.unit, "sat");
        assert_eq!(balance.balance_sat, 0);
    }

    #[tokio::test]
    async fn test_create_topup_quote_rejects_zero_amount() {
        let temp_dir = tempfile::tempdir().unwrap();
        let err = create_topup_quote(temp_dir.path(), "https://mint.example", 0)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("greater than zero"));
    }

    #[tokio::test]
    async fn test_send_payment_token_rejects_zero_amount() {
        let temp_dir = tempfile::tempdir().unwrap();
        let err = send_payment_token(temp_dir.path(), "https://mint.example", 0)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("greater than zero"));
    }

    #[tokio::test]
    async fn test_cross_mint_transfer_pays_and_issues_exact_destination_quote_once() {
        let state = Arc::new(CrossMintTestState::default());
        let (source, destination) = cross_mint_test_wallets(2, state.clone()).await;
        let request = cross_mint_request("transfer_1");

        let first = transfer_between_wallets(&source, &destination, request.clone())
            .await
            .unwrap();
        let retry = transfer_between_wallets(&source, &destination, request)
            .await
            .unwrap();

        assert_eq!(retry, first);
        assert_eq!(first.amount_sat, 250_000);
        assert_eq!(first.melt_fee_reserve_sat, 2);
        assert_eq!(first.wallet_fee_sat, 0);
        assert_eq!(first.fee_paid_sat, 2);
        assert_eq!(first.source_melt_quote_id, "source-melt-quote");
        assert_eq!(first.destination_mint_quote_id, "destination-mint-quote");
        assert_eq!(first.destination_balance_before_sat, 0);
        assert_eq!(first.destination_balance_after_sat, 250_000);
        assert_eq!(state.mint_quote_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.melt_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.mint_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_cross_mint_transfer_recovers_after_payment_before_destination_refresh() {
        let state = Arc::new(CrossMintTestState::default());
        state.delay_destination_once.store(true, Ordering::SeqCst);
        let (source, destination) = cross_mint_test_wallets(2, state.clone()).await;
        let request = cross_mint_request("transfer_recovery");

        let interrupted = transfer_between_wallets(&source, &destination, request.clone())
            .await
            .unwrap_err();
        assert!(interrupted.to_string().contains("unexpected state"));
        assert_eq!(state.melt_calls.load(Ordering::SeqCst), 1);
        assert_eq!(destination.total_balance().await.unwrap().to_u64(), 0);

        let recovered = transfer_between_wallets(&source, &destination, request)
            .await
            .unwrap();
        assert_eq!(recovered.destination_balance_after_sat, 250_000);
        assert_eq!(state.mint_quote_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.melt_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.mint_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_cross_mint_transfer_rejects_fee_before_payment() {
        let state = Arc::new(CrossMintTestState::default());
        let (source, destination) = cross_mint_test_wallets(3, state.clone()).await;
        let request = cross_mint_request("transfer_fee_limit");

        let error = transfer_between_wallets(&source, &destination, request)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("exceeds caller maximum"));
        assert_eq!(state.melt_calls.load(Ordering::SeqCst), 0);
        assert_eq!(state.mint_calls.load(Ordering::SeqCst), 0);
        assert_eq!(destination.total_balance().await.unwrap().to_u64(), 0);
    }

    #[tokio::test]
    async fn test_cross_mint_transfer_rejects_wrong_invoice_amount_before_payment() {
        let state = Arc::new(CrossMintTestState::default());
        let (source, destination) = cross_mint_test_wallets(2, state.clone()).await;
        let mut request = cross_mint_request("transfer_wrong_amount");
        request.amount_sat = 249_999;

        let error = transfer_between_wallets(&source, &destination, request)
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("does not match requested amount"));
        assert_eq!(state.melt_calls.load(Ordering::SeqCst), 0);
        assert_eq!(state.mint_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_cross_mint_transfer_id_cannot_be_rebound() {
        let state = Arc::new(CrossMintTestState::default());
        let (source, destination) = cross_mint_test_wallets(2, state.clone()).await;
        let request = cross_mint_request("transfer_bound");
        transfer_between_wallets(&source, &destination, request.clone())
            .await
            .unwrap();
        let mut changed = request;
        changed.max_fee_sat = 3;

        let error = transfer_between_wallets(&source, &destination, changed)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("bound to a different request"));
        assert_eq!(state.melt_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_send_lightning_payment_with_wallet_returns_preimage() {
        let keyset = build_test_keyset(250_000);
        let mint_url: MintUrl = "https://mint.example".parse().unwrap();
        let proof_infos = binary_proof_infos(mint_url.clone(), keyset.id, 250_000);
        let db = cdk_sqlite::wallet::memory::empty().await.unwrap();
        db.update_proofs(proof_infos, vec![]).await.unwrap();

        let mock = Arc::new(LightningMockMintConnector::new(keyset, "quote-123", "00ff"));
        let wallet = WalletBuilder::new()
            .mint_url(mint_url)
            .unit(CurrencyUnit::Sat)
            .localstore(Arc::new(db))
            .seed([7_u8; 64])
            .shared_client(mock)
            .build()
            .unwrap();

        let payment = send_lightning_payment_with_wallet(&wallet, K_VALID_BOLT11_INVOICE)
            .await
            .unwrap();
        assert_eq!(payment.mint_url, "https://mint.example");
        assert_eq!(payment.unit, "sat");
        assert_eq!(payment.amount_sat, 250_000);
        assert_eq!(payment.fee_paid_sat, 0);
        assert_eq!(payment.quote_id, "quote-123");
        assert_eq!(payment.preimage, "00ff");
    }

    #[tokio::test]
    async fn test_receive_payment_token_rejects_invalid_token() {
        let temp_dir = tempfile::tempdir().unwrap();
        let err = receive_payment_token(temp_dir.path(), "not-a-token")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("parse Cashu token"));
    }

    #[tokio::test]
    async fn test_import_payment_proofs_adds_spendable_balance() {
        let temp_dir = tempfile::tempdir().unwrap();
        let keyset = build_test_keyset(16);
        let proofs = vec![make_test_proof(keyset.id, 4), make_test_proof(keyset.id, 8)];
        let proofs_json = serde_json::to_string(&proofs).unwrap();

        let imported = import_payment_proofs(
            temp_dir.path(),
            "https://mint.example/",
            "sat",
            &proofs_json,
        )
        .await
        .unwrap();

        assert_eq!(imported.mint_url, "https://mint.example");
        assert_eq!(imported.unit, "sat");
        assert_eq!(imported.amount_sat, 12);

        let overview = load_wallet_overview(temp_dir.path(), false).await.unwrap();
        assert_eq!(
            overview.entries,
            vec![CashuWalletEntry {
                mint_url: "https://mint.example".to_string(),
                unit: "sat".to_string(),
                balance: 12,
            }]
        );

        let activity = load_wallet_activity(temp_dir.path()).await.unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].kind, CashuWalletActivityKind::ChannelCollect);
        assert_eq!(activity[0].amount_sat, 12);
    }

    #[tokio::test]
    async fn test_import_payment_proofs_is_idempotent() {
        let temp_dir = tempfile::tempdir().unwrap();
        let keyset = build_test_keyset(16);
        let proofs = vec![make_test_proof(keyset.id, 4), make_test_proof(keyset.id, 8)];
        let proofs_json = serde_json::to_string(&proofs).unwrap();

        let first =
            import_payment_proofs(temp_dir.path(), "https://mint.example", "sat", &proofs_json)
                .await
                .unwrap();
        let second =
            import_payment_proofs(temp_dir.path(), "https://mint.example", "sat", &proofs_json)
                .await
                .unwrap();

        assert_eq!(first.amount_sat, 12);
        assert_eq!(second.amount_sat, 0);
        let overview = load_wallet_overview(temp_dir.path(), false).await.unwrap();
        assert_eq!(overview.entries[0].balance, 12);
        let activity = load_wallet_activity(temp_dir.path()).await.unwrap();
        assert_eq!(activity.len(), 1);
    }

    #[tokio::test]
    async fn test_revoke_pending_payment_rejects_invalid_operation_id() {
        let temp_dir = tempfile::tempdir().unwrap();
        let err = revoke_pending_payment(temp_dir.path(), "https://mint.example", "nope")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Invalid Cashu send operation id"));
    }

    #[tokio::test]
    async fn test_wallet_activity_pending_topup_syncs_to_complete() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mint_url: MintUrl = "https://mint.example".parse().unwrap();
        let repository = open_wallet_repository(temp_dir.path()).await.unwrap();
        let wallet = ensure_sat_wallet(&repository, &mint_url).await.unwrap();

        let mut quote = cdk::wallet::MintQuote::new(
            "quote-1".to_string(),
            mint_url.clone(),
            PaymentMethod::BOLT11,
            Some(Amount::from(5_u64)),
            CurrencyUnit::Sat,
            "lnbc5n1p0test".to_string(),
            wallet_activity_now_unix() + 300,
            None,
        );
        quote.state = MintQuoteState::Issued;
        wallet.localstore.add_mint_quote(quote).await.unwrap();

        append_wallet_activity_entry(
            temp_dir.path(),
            CashuWalletActivityEntry {
                id: "entry-topup".to_string(),
                kind: CashuWalletActivityKind::TopUp,
                status: CashuWalletActivityStatus::Pending,
                mint_url: mint_url.to_string(),
                unit: "sat".to_string(),
                amount_sat: 5,
                fee_sat: None,
                created_at_unix: wallet_activity_now_unix(),
                expires_at_unix: Some(wallet_activity_now_unix() + 300),
                quote_id: Some("quote-1".to_string()),
                operation_id: None,
                payment_request: Some("lnbc5n1p0test".to_string()),
                token: None,
            },
        )
        .await
        .unwrap();

        let history = load_wallet_activity(temp_dir.path()).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, CashuWalletActivityStatus::Complete);
    }

    #[tokio::test]
    async fn test_wallet_activity_pending_send_syncs_to_complete_without_saga() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mint_url: MintUrl = "https://mint.example".parse().unwrap();
        let repository = open_wallet_repository(temp_dir.path()).await.unwrap();
        ensure_sat_wallet(&repository, &mint_url).await.unwrap();

        append_wallet_activity_entry(
            temp_dir.path(),
            CashuWalletActivityEntry {
                id: "entry-send".to_string(),
                kind: CashuWalletActivityKind::TokenSend,
                status: CashuWalletActivityStatus::Pending,
                mint_url: mint_url.to_string(),
                unit: "sat".to_string(),
                amount_sat: 3,
                fee_sat: Some(1),
                created_at_unix: wallet_activity_now_unix(),
                expires_at_unix: None,
                quote_id: None,
                operation_id: Some(Uuid::new_v4().to_string()),
                payment_request: None,
                token: Some("cashuBtoken".to_string()),
            },
        )
        .await
        .unwrap();

        let history = load_wallet_activity(temp_dir.path()).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, CashuWalletActivityStatus::Complete);
    }

    #[tokio::test]
    async fn test_create_topup_quote_against_configured_mint() {
        let mint_url = std::env::var("CASHU_SERVICE_TEST_MINT_URL")
            .or_else(|_| std::env::var("HTREE_CASHU_TEST_MINT_URL"));
        let Ok(mint_url) = mint_url else { return };

        let temp_dir = tempfile::tempdir().unwrap();
        let quote = create_topup_quote(temp_dir.path(), &mint_url, 1)
            .await
            .unwrap();
        assert_eq!(quote.amount, 1);
        assert_eq!(quote.unit, "sat");
        assert!(!quote.quote_id.is_empty());
        assert!(!quote.payment_request.is_empty());
    }
}
