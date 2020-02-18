table! {
    coinbase_transactions (tx_id) {
        tx_id -> BigInt,
        amount -> BigInt,
        commitment -> Binary,
        timestamp -> Timestamp,
    }
}

table! {
    completed_transactions (tx_id) {
        tx_id -> BigInt,
        source_node_id -> Binary,
        destination_node_id -> Binary,
        amount -> BigInt,
        fee -> BigInt,
        transaction_protocol -> Text,
        status -> Integer,
        message -> Text,
        timestamp -> Timestamp,
    }
}

table! {
    contacts (node_id) {
        node_id -> Binary,
        alias -> Text,
    }
}

table! {
    inbound_transactions (tx_id) {
        tx_id -> BigInt,
        source_node_id -> Binary,
        amount -> BigInt,
        receiver_protocol -> Text,
        message -> Text,
        timestamp -> Timestamp,
    }
}

table! {
    key_manager_states (id) {
        id -> Nullable<BigInt>,
        master_seed -> Binary,
        branch_seed -> Text,
        primary_key_index -> BigInt,
        timestamp -> Timestamp,
    }
}

table! {
    outbound_transactions (tx_id) {
        tx_id -> BigInt,
        destination_node_id -> Binary,
        amount -> BigInt,
        fee -> BigInt,
        sender_protocol -> Text,
        message -> Text,
        timestamp -> Timestamp,
    }
}

table! {
    outputs (spending_key) {
        spending_key -> Binary,
        value -> BigInt,
        flags -> Integer,
        maturity -> BigInt,
        status -> Integer,
        tx_id -> Nullable<BigInt>,
    }
}

table! {
    peers (public_key) {
        public_key -> Binary,
        peer -> Text,
    }
}

table! {
    pending_transaction_outputs (tx_id) {
        tx_id -> BigInt,
        timestamp -> Timestamp,
    }
}

joinable!(outputs -> pending_transaction_outputs (tx_id));

allow_tables_to_appear_in_same_query!(
    coinbase_transactions,
    completed_transactions,
    contacts,
    inbound_transactions,
    key_manager_states,
    outbound_transactions,
    outputs,
    peers,
    pending_transaction_outputs,
);
