table! {
    dht_settings (id) {
        id -> Integer,
        key -> Text,
        value -> Binary,
    }
}

table! {
    stored_messages (id) {
        id -> Integer,
        version -> Integer,
        origin_pubkey -> Nullable<Text>,
        message_type -> Integer,
        destination_pubkey -> Nullable<Text>,
        destination_node_id -> Nullable<Text>,
        header -> Binary,
        body -> Binary,
        is_encrypted -> Bool,
        priority -> Integer,
        stored_at -> Timestamp,
    }
}

allow_tables_to_appear_in_same_query!(dht_settings, stored_messages,);
