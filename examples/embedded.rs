/// Example demonstrating Minigraf as an embedded graph database.
///
/// This example shows how to use Minigraf like SQLite - as an embedded
/// database with a single `.graph` file that persists across restarts.
use minigraf::Minigraf;

fn main() -> anyhow::Result<()> {
    println!("=== Minigraf Embedded Database Example ===\n");

    let db_path = "/tmp/social_network.graph";

    // Clean up any existing database
    let _ = std::fs::remove_file(db_path);

    // Part 1: Create a social network graph
    println!("Part 1: Creating a social network...");
    {
        let mut db = Minigraf::open(db_path)?;
        println!("✓ Opened database: {}", db_path);

        // Create people
        db.execute("CREATE NODE (:Person) {name: \"Alice\", age: 30, city: \"NYC\"}")?;
        db.execute("CREATE NODE (:Person) {name: \"Bob\", age: 25, city: \"SF\"}")?;
        db.execute("CREATE NODE (:Person) {name: \"Charlie\", age: 35, city: \"LA\"}")?;
        println!("✓ Created 3 people");

        // Get the node IDs to create relationships
        let result = db.execute("SHOW NODES")?;
        let node_ids: Vec<String> = if let minigraf::query::executor::QueryResult::Nodes(nodes) = result {
            nodes.iter().map(|n| n.id.clone()).collect()
        } else {
            vec![]
        };

        if node_ids.len() >= 3 {
            // Create friendships
            let alice_id = &node_ids[0];
            let bob_id = &node_ids[1];
            let charlie_id = &node_ids[2];

            db.execute(&format!(
                "CREATE EDGE ({})-[KNOWS]->({}) {{since: 2020}}",
                alice_id, bob_id
            ))?;
            db.execute(&format!(
                "CREATE EDGE ({})-[KNOWS]->({}) {{since: 2019}}",
                bob_id, charlie_id
            ))?;
            db.execute(&format!(
                "CREATE EDGE ({})-[KNOWS]->({}) {{since: 2021}}",
                charlie_id, alice_id
            ))?;
            println!("✓ Created 3 friendships");
        }

        let stats = db.stats();
        println!(
            "✓ Database stats: {} nodes, {} edges",
            stats.node_count, stats.edge_count
        );

        // Database automatically persists on drop
        println!("✓ Database will auto-save on close...");
    }

    println!("\n--- Database closed ---\n");

    // Part 2: Reopen and query
    println!("Part 2: Reopening database (demonstrating persistence)...");
    {
        let mut db = Minigraf::open(db_path)?;
        println!("✓ Reopened database: {}", db_path);

        let stats = db.stats();
        println!(
            "✓ Data persisted! {} nodes, {} edges still there",
            stats.node_count, stats.edge_count
        );

        // Query people
        println!("\nQuerying all people:");
        let result = db.execute("MATCH (:Person)")?;
        if let minigraf::query::executor::QueryResult::Nodes(nodes) = result {
            for node in nodes {
                let name = node
                    .properties
                    .get("name")
                    .and_then(|v| v.as_string())
                    .unwrap_or("Unknown");
                let age = node
                    .properties
                    .get("age")
                    .and_then(|v| v.as_integer())
                    .unwrap_or(0);
                println!("  - {} (age {})", name, age);
            }
        }

        // Query relationships
        println!("\nQuerying all friendships:");
        let result = db.execute("MATCH -[:KNOWS]->")?;
        if let minigraf::query::executor::QueryResult::Edges(edges) = result {
            println!("  Found {} friendship connections", edges.len());
        }

        // Query with filter
        println!("\nQuerying people named Alice:");
        let result = db.execute("MATCH (:Person) WHERE name = \"Alice\"")?;
        if let minigraf::query::executor::QueryResult::Nodes(nodes) = result {
            println!("  Found {} person(s)", nodes.len());
        }
    }

    // Part 3: Add more data and verify
    println!("\nPart 3: Adding more data...");
    {
        let mut db = Minigraf::open(db_path)?;
        db.execute("CREATE NODE (:Person) {name: \"Diana\", age: 28}")?;
        println!("✓ Added Diana");

        db.save()?; // Explicit save (also happens automatically on drop)
        println!("✓ Explicitly saved changes");

        let stats = db.stats();
        println!("✓ Now have {} nodes", stats.node_count);
    }

    // Final verification
    println!("\nFinal verification:");
    {
        let db = Minigraf::open(db_path)?;
        let stats = db.stats();
        println!(
            "✓ Final database state: {} nodes, {} edges",
            stats.node_count, stats.edge_count
        );
    }

    // Clean up
    std::fs::remove_file(db_path)?;
    println!("\n✓ Cleaned up example database");

    println!("\n=== Example Complete! ===");
    println!("\nKey Takeaways:");
    println!("1. Minigraf::open() creates or opens a .graph file");
    println!("2. Execute queries with .execute()");
    println!("3. Changes auto-persist when database is dropped");
    println!("4. Reopen the same file to access persisted data");
    println!("5. Works like SQLite - one file, embedded in your app!");

    Ok(())
}
