use minigraf::{GraphStorage, parse_query, QueryExecutor};

#[test]
fn test_complete_workflow() {
    let storage = GraphStorage::new();
    let executor = QueryExecutor::new(&storage);

    // Create nodes
    let create_alice = parse_query("CREATE NODE (:Person) {name: \"Alice\", age: 30}").unwrap();
    let result = executor.execute(create_alice).unwrap();

    let alice_id = match result {
        minigraf::query::executor::QueryResult::NodeCreated(node) => {
            println!("Created Alice: {}", node.id);
            node.id
        }
        _ => panic!("Expected NodeCreated"),
    };

    let create_bob = parse_query("CREATE NODE (:Person:Employee) {name: \"Bob\", age: 25}").unwrap();
    let result = executor.execute(create_bob).unwrap();

    let bob_id = match result {
        minigraf::query::executor::QueryResult::NodeCreated(node) => {
            println!("Created Bob: {}", node.id);
            node.id
        }
        _ => panic!("Expected NodeCreated"),
    };

    // Show all nodes
    let show_nodes = parse_query("SHOW NODES").unwrap();
    let result = executor.execute(show_nodes).unwrap();
    println!("\n{}", result.format());

    // Create edge
    let create_edge_query = format!("CREATE EDGE ({})-[KNOWS]->({}) {{since: 2020}}", alice_id, bob_id);
    let create_edge = parse_query(&create_edge_query).unwrap();
    let result = executor.execute(create_edge).unwrap();
    println!("\n{}", result.format());

    // Show all edges
    let show_edges = parse_query("SHOW EDGES").unwrap();
    let result = executor.execute(show_edges).unwrap();
    println!("\n{}", result.format());

    // Match Person nodes
    let match_persons = parse_query("MATCH (:Person)").unwrap();
    let result = executor.execute(match_persons).unwrap();
    println!("\n{}", result.format());

    // Match with filter
    let match_alice = parse_query("MATCH (:Person) WHERE name = \"Alice\"").unwrap();
    let result = executor.execute(match_alice).unwrap();
    println!("\n{}", result.format());
}
