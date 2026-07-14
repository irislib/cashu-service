use super::*;
use async_trait::async_trait;
use cdk::cdk_database::WalletDatabase;
use cdk::nuts::{
    BatchCheckMintQuoteRequest, BatchMintRequest, CheckStateRequest, CheckStateResponse,
    CurrencyUnit, Id, KeySet, KeySetInfo, Keys, KeysetResponse, MeltQuoteBolt11Response,
    MeltRequest, MintInfo, MintQuoteBolt11Response, MintRequest, MintResponse, PaymentMethod,
    Proof, RestoreRequest, RestoreResponse, SecretKey, State, SwapRequest, SwapResponse,
};
use cdk::secret::Secret;
use cdk::wallet::{types::ProofInfo, MintConnector, WalletBuilder};
use cdk::{Amount, Error};
use cdk_common::{MeltQuoteRequest, MeltQuoteResponse, MintQuoteRequest, MintQuoteResponse};
use std::collections::BTreeMap;
use std::sync::Arc;

const K_VALID_BOLT11_INVOICE: &str = "lnbc2500u1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygspp5qqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqdq5xysxxatsyp3k7enxv4jsxqzpu9qrsgquk0rl77nj30yxdy8j9vdx85fkpmdla2087ne0xh8nhedh8w27kyke0lp53ut353s06fv3qfegext0eh0ymjpf39tuven09sam30g4vgpfna3rh";
const K_TEST_EXPIRY_UNIX: u64 = 4_102_444_800;

#[derive(Debug, Clone)]
struct LightningMockMintConnector {
    keyset: KeySet,
    quote_id: String,
    preimage: String,
}

impl LightningMockMintConnector {
    fn new(keyset: KeySet, quote_id: &str, preimage: &str) -> Self {
        Self {
            keyset,
            quote_id: quote_id.to_string(),
            preimage: preimage.to_string(),
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
        _request: MintQuoteRequest,
    ) -> Result<MintQuoteResponse<String>, Error> {
        unreachable!("unused in Cashu Lightning payment test")
    }

    async fn get_mint_quote_status(
        &self,
        _method: PaymentMethod,
        _quote_id: &str,
    ) -> Result<MintQuoteResponse<String>, Error> {
        unreachable!("unused in Cashu Lightning payment test")
    }

    async fn post_mint(
        &self,
        _method: &PaymentMethod,
        _request: MintRequest<String>,
    ) -> Result<MintResponse, Error> {
        unreachable!("unused in Cashu Lightning payment test")
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
            fee_reserve: Amount::ZERO,
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
        _quote_id: &str,
    ) -> Result<MeltQuoteResponse<String>, Error> {
        unreachable!("unused in Cashu Lightning payment test")
    }

    async fn post_melt(
        &self,
        _method: &PaymentMethod,
        request: MeltRequest<String>,
    ) -> Result<MeltQuoteBolt11Response<String>, Error> {
        if request.quote_id() != &self.quote_id {
            return Err(Error::Custom("unexpected quote id".to_string()));
        }
        Ok(MeltQuoteBolt11Response {
            quote: self.quote_id.clone(),
            amount: Amount::from(250_000_u64),
            fee_reserve: Amount::ZERO,
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

    let first = import_payment_proofs(temp_dir.path(), "https://mint.example", "sat", &proofs_json)
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
