mod yt_stream_seek;
mod seek_streaming;
mod mock_youtube_stream;
mod real_youtube_stream;

use real_youtube_stream::{
    test_backward_seek_with_cache, test_forward_seek_with_cache,
    test_real_youtube_stream_with_cache, test_seek_accuracy,
};

#[tokio::test]
async fn test_suite_real_youtube_stream() {
    println!("╔════════════════════════════════════════════════════════════════════╗");
    println!("║              REAL YOUTUBE STREAM TEST SUITE                        ║");
    println!("╚════════════════════════════════════════════════════════════════════╝\n");

    println!("Starting real YouTube stream tests with captured data...\n");

    let mut tests_passed = 0;
    let mut tests_failed = 0;

    let tests = vec![
        ("Stream Reading with Cache", || {
            println!("\n=== Test 1: Stream Reading with Cache ===");
            let result = test_real_youtube_stream_with_cache();
            match result {
                Ok(_) => {
                    println!("✓ Stream Reading Test PASSED");
                    Ok(())
                }
                Err(e) => {
                    println!("✗ Stream Reading Test FAILED: {}", e);
                    Err(e)
                }
            }
        }),
        ("Backward Seek with Cache", || {
            println!("\n=== Test 2: Backward Seek with Cache ===");
            let result = test_backward_seek_with_cache();
            match result {
                Ok(_) => {
                    println!("✓ Backward Seek Test PASSED");
                    Ok(())
                }
                Err(e) => {
                    println!("✗ Backward Seek Test FAILED: {}", e);
                    Err(e)
                }
            }
        }),
        ("Forward Seek with Cache", || {
            println!("\n=== Test 3: Forward Seek with Cache ===");
            let result = test_forward_seek_with_cache();
            match result {
                Ok(_) => {
                    println!("✓ Forward Seek Test PASSED");
                    Ok(())
                }
                Err(e) => {
                    println!("✗ Forward Seek Test FAILED: {}", e);
                    Err(e)
                }
            }
        }),
        ("Seek Accuracy", || {
            println!("\n=== Test 4: Seek Accuracy ===");
            let result = test_seek_accuracy();
            match result {
                Ok(_) => {
                    println!("✓ Seek Accuracy Test PASSED");
                    Ok(())
                }
                Err(e) => {
                    println!("✗ Seek Accuracy Test FAILED: {}", e);
                    Err(e)
                }
            }
        }),
    ];

    for (i, (name, test_fn)) in tests.into_iter().enumerate() {
        println!("Running test {}/{}", i + 1, tests.len());
        println!("┌──────────────────────────────────────────────────────────────────┐");
        
        if let Err(e) = test_fn() {
            tests_failed += 1;
            println!("└──────────────────────────────────────────────────────────────────┘");
            continue;
        }
        
        tests_passed += 1;
        println!("└──────────────────────────────────────────────────────────────────┘");
    }

    println!("\n╔════════════════════════════════════════════════════════════════════╗");
    println!("║                        TEST SUMMARY                                ║");
    println!("╠════════════════════════════════════════════════════════════════════╣");
    println!("║  Tests Passed: {} ✓                                                  ║", tests_passed);
    println!("║  Tests Failed: {} ✗                                                  ║", tests_failed);
    println!("╠════════════════════════════════════════════════════════════════════╣");
    
    if tests_failed == 0 {
        println!("║  Overall Status: ALL TESTS PASSED! 🎉                              ║");
    } else {
        println!("║  Overall Status: SOME TESTS FAILED                                  ║");
    }
    println!("╚════════════════════════════════════════════════════════════════════╝\n");

    assert_eq!(tests_failed, 0, "Some tests failed");
}
