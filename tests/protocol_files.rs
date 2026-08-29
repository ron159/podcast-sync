use serde_json::Value;

#[test]
fn protocol_schemas_and_examples_are_valid_json() {
    for source in [
        include_str!("../protocol/schemas/sync-request.schema.json"),
        include_str!("../protocol/schemas/sync-response.schema.json"),
        include_str!("../protocol/schemas/snapshot.schema.json"),
        include_str!("../protocol/examples/android-push.json"),
        include_str!("../protocol/examples/ios-pull-response.json"),
    ] {
        serde_json::from_str::<Value>(source).unwrap();
    }
}
