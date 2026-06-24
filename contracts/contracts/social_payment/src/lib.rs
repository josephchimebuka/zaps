#![no_std]
#![allow(dead_code, unused_variables, unused_imports, unexpected_cfgs)]
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, String, Symbol};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Visibility {
    Public = 0,
    Friends = 1,
    Private = 2,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SocialPaymentEvent {
    pub sender: Address,
    pub receiver: Address,
    pub amount: i128,
    pub memo: String,
    pub visibility: Visibility,
}

#[contract]
pub struct SocialPaymentContract;

#[contractimpl]
impl SocialPaymentContract {
    /// Initialize the contract with admin and treasury addresses
    pub fn initialize(env: Env, admin: Address, treasury: Address) {
        if env.storage().instance().has(&Symbol::new(&env, "admin")) {
            panic!("already initialized");
        }
        env.storage().instance().set(&Symbol::new(&env, "admin"), &admin);
        env.storage().instance().set(&Symbol::new(&env, "treasury"), &treasury);
    }

    /// Set a new treasury address (admin only)
    pub fn set_treasury(env: Env, new_treasury: Address) {
        let admin: Address = env.storage().instance().get(&Symbol::new(&env, "admin"))
            .unwrap_or_else(|| panic!("not initialized"));
        admin.require_auth();
        env.storage().instance().set(&Symbol::new(&env, "treasury"), &new_treasury);
    }

    /// Execute a social payment between two users
    pub fn pay(
        env: Env,
        sender: Address,
        receiver: Address,
        token: Address,
        amount: i128,
        memo: String,
        visibility: Visibility,
    ) {
        sender.require_auth();
        if amount <= 0 {
            panic!("amount must be positive");
        }

        if visibility == Visibility::Public {
            let treasury: Address = env.storage().instance().get(&Symbol::new(&env, "treasury"))
                .unwrap_or_else(|| panic!("treasury not initialized"));
            let fee = amount / 1000;
            let receiver_amount = amount - fee;

            let token_client = soroban_sdk::token::Client::new(&env, &token);
            token_client.transfer(&sender, &receiver, &receiver_amount);
            if fee > 0 {
                token_client.transfer(&sender, &treasury, &fee);
            }
        } else {
            let token_client = soroban_sdk::token::Client::new(&env, &token);
            token_client.transfer(&sender, &receiver, &amount);
        }

        env.events().publish(
            (Symbol::new(&env, "SocialPaymentEvent"),),
            SocialPaymentEvent {
                sender,
                receiver,
                amount,
                memo,
                visibility,
            },
        );
    }

    /// Add a like to a transaction (on-chain action or registry log)
    pub fn like_payment(env: Env, sender: Address, tx_id: Symbol) {
        sender.require_auth();
        env.events().publish(
            (Symbol::new(&env, "PaymentLiked"),),
            (tx_id, sender),
        );
    }

    /// Add a comment to a transaction (on-chain event trigger)
    pub fn comment_payment(env: Env, sender: Address, tx_id: Symbol, comment: String) {
        sender.require_auth();
        let len = comment.len();
        if len > 120 {
            panic!("comment exceeds maximum length of 120 characters");
        }
        env.events().publish(
            (Symbol::new(&env, "PaymentCommented"),),
            (tx_id, comment.clone()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Events},
        Address, Env, String, Symbol, IntoVal, TryIntoVal, Val,
    };

    #[test]
    fn comment_payment_accepts_valid_comment() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, SocialPaymentContract);
        let client = SocialPaymentContractClient::new(&env, &contract_id);

        let sender = Address::generate(&env);
        let tx_id = Symbol::new(&env, "payment-123");
        let comment = String::from_str(&env, "This is a valid comment.");

        client.comment_payment(&sender, &tx_id, &comment);
    }

    #[test]
    #[should_panic(expected = "comment exceeds maximum length")]
    fn comment_payment_rejects_overlong_comment() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, SocialPaymentContract);
        let client = SocialPaymentContractClient::new(&env, &contract_id);

        let sender = Address::generate(&env);
        let tx_id = Symbol::new(&env, "payment-456");
        let long_text = "x".repeat(121);
        let comment = String::from_str(&env, &long_text);

        client.comment_payment(&sender, &tx_id, &comment);
    }

    #[test]
    fn test_social_payment_public_visibility_deducts_fee() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, SocialPaymentContract);
        let client = SocialPaymentContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let receiver = Address::generate(&env);

        // Initialize contract
        client.initialize(&admin, &treasury);

        // Register standard token contract
        let token_address = env.register_stellar_asset_contract(admin.clone());
        let token_client = soroban_sdk::token::Client::new(&env, &token_address);
        let token_admin = soroban_sdk::token::StellarAssetClient::new(&env, &token_address);

        // Mint tokens to sender
        token_admin.mint(&sender, &10000);

        let amount = 1000i128;
        let memo = String::from_str(&env, "Public payment");

        client.pay(&sender, &receiver, &token_address, &amount, &memo, &Visibility::Public);

        // Platform fee = 0.1% of 1000 = 1
        // Receiver should get 999, Treasury 1, Sender balance should be 9000
        assert_eq!(token_client.balance(&receiver), 999);
        assert_eq!(token_client.balance(&treasury), 1);
        assert_eq!(token_client.balance(&sender), 9000);

        // Check event emission
        let events = env.events().all();
        assert!(events.len() > 0);
        
        // Find the SocialPaymentEvent
        let mut found = false;
        let topic: Val = Symbol::new(&env, "SocialPaymentEvent").into_val(&env);
        for item in events.iter() {
            if item.1.contains(topic.clone()) {
                let event: SocialPaymentEvent = item.2.try_into_val(&env).unwrap();
                assert_eq!(event.sender, sender);
                assert_eq!(event.receiver, receiver);
                assert_eq!(event.amount, amount);
                assert_eq!(event.memo, memo);
                assert_eq!(event.visibility, Visibility::Public);
                found = true;
                break;
            }
        }
        assert!(found, "SocialPaymentEvent was not emitted");
    }

    #[test]
    fn test_social_payment_private_visibility_no_fee() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, SocialPaymentContract);
        let client = SocialPaymentContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let receiver = Address::generate(&env);

        client.initialize(&admin, &treasury);

        let token_address = env.register_stellar_asset_contract(admin.clone());
        let token_client = soroban_sdk::token::Client::new(&env, &token_address);
        let token_admin = soroban_sdk::token::StellarAssetClient::new(&env, &token_address);

        token_admin.mint(&sender, &10000);

        let amount = 1000i128;
        let memo = String::from_str(&env, "Private payment");

        client.pay(&sender, &receiver, &token_address, &amount, &memo, &Visibility::Private);

        // Receiver should get full 1000, Treasury 0, Sender balance 9000
        assert_eq!(token_client.balance(&receiver), 1000);
        assert_eq!(token_client.balance(&treasury), 0);
        assert_eq!(token_client.balance(&sender), 9000);
    }

    #[test]
    fn test_like_payment_emits_event() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, SocialPaymentContract);
        let client = SocialPaymentContractClient::new(&env, &contract_id);

        let sender = Address::generate(&env);
        let tx_id = Symbol::new(&env, "tx-hash-456");

        client.like_payment(&sender, &tx_id);

        // Check event emission
        let events = env.events().all();
        assert!(events.len() > 0);

        let mut found = false;
        let topic: Val = Symbol::new(&env, "PaymentLiked").into_val(&env);
        for item in events.iter() {
            if item.1.contains(topic.clone()) {
                let (event_tx_id, event_sender): (Symbol, Address) = item.2.try_into_val(&env).unwrap();
                assert_eq!(event_tx_id, tx_id);
                assert_eq!(event_sender, sender);
                found = true;
                break;
            }
        }
        assert!(found, "PaymentLiked event was not emitted");
    }
}
