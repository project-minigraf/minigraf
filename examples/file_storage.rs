/// Example demonstrating the file-based storage backend.
///
/// This example shows how to use the low-level storage backend API
/// to read and write pages to a .graph file.

use minigraf::storage::backend::{FileBackend, MemoryBackend};
use minigraf::storage::{FileHeader, StorageBackend, PAGE_SIZE};

fn main() -> anyhow::Result<()> {
    println!("Minigraf Storage Backend Example\n");

    // Example 1: In-memory storage
    println!("=== In-Memory Storage ===");
    demo_memory_backend()?;

    // Example 2: File-based storage
    println!("\n=== File-Based Storage ===");
    demo_file_backend()?;

    println!("\n✅ All storage backend examples completed!");
    Ok(())
}

fn demo_memory_backend() -> anyhow::Result<()> {
    let mut backend = MemoryBackend::new();
    println!("Created {} backend", backend.backend_name());

    // Write some data
    let data = b"Hello from Minigraf memory storage!".to_vec();
    let mut page = vec![0u8; PAGE_SIZE];
    page[..data.len()].copy_from_slice(&data);

    backend.write_page(0, &page)?;
    println!("Wrote {} bytes to page 0", data.len());

    // Read it back
    let read_page = backend.read_page(0)?;
    let read_data = &read_page[..data.len()];
    println!("Read back: {}", String::from_utf8_lossy(read_data));

    println!("Total pages: {}", backend.page_count()?);

    Ok(())
}

fn demo_file_backend() -> anyhow::Result<()> {
    let path = "/tmp/example.graph";

    // Create a new .graph file
    {
        let mut backend = FileBackend::open(path)?;
        println!("Created {} backend at: {}", backend.backend_name(), path);

        // Check the header
        let page_count = backend.page_count()?;
        println!("Initial page count: {}", page_count);

        // Write some data to page 1 (page 0 is the header)
        let data = b"Hello from Minigraf file storage!".to_vec();
        let mut page = vec![0u8; PAGE_SIZE];
        page[..data.len()].copy_from_slice(&data);

        backend.write_page(1, &page)?;
        println!("Wrote {} bytes to page 1", data.len());

        // Write another page
        let data2 = b"This is page 2 with more data!".to_vec();
        let mut page2 = vec![0u8; PAGE_SIZE];
        page2[..data2.len()].copy_from_slice(&data2);

        backend.write_page(2, &page2)?;
        println!("Wrote {} bytes to page 2", data2.len());

        // Sync to ensure data is written
        backend.sync()?;
        println!("Synced data to disk");

        println!("Total pages: {}", backend.page_count()?);

        // Close explicitly (also happens on drop)
        backend.close()?;
        println!("Closed file");
    }

    // Reopen and read the data back
    {
        println!("\nReopening file to verify persistence...");
        let backend = FileBackend::open(path)?;

        // Read back the data
        let page1 = backend.read_page(1)?;
        let data1 = String::from_utf8_lossy(&page1[..33]);
        println!("Read from page 1: {}", data1);

        let page2 = backend.read_page(2)?;
        let data2 = String::from_utf8_lossy(&page2[..31]);
        println!("Read from page 2: {}", data2);

        println!("Total pages: {}", backend.page_count()?);
    }

    // Clean up
    std::fs::remove_file(path)?;
    println!("\nCleaned up example file");

    Ok(())
}
