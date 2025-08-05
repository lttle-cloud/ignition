async function main() {
  console.log('=== Testing HTTP Requests ===\n');

  // Test 1: HTTP request to nginx-test.default.svc.lttle.local
  console.log('1. Testing nginx-test.default.svc.lttle.local:');
  try {
    const response = await fetch('http://nginx-test.default.svc.lttle.local', {
      signal: AbortSignal.timeout(5000)
    });
    console.log(`✓ Connected - Status: ${response.status}`);
  } catch (error) {
    console.log(`✗ Failed:`, error.message);
  }

  // Test 2: HTTP request to google.com
  console.log('\n2. Testing google.com:');
  try {
    const response = await fetch('https://google.com', {
      signal: AbortSignal.timeout(5000)
    });
    console.log(`✓ Connected - Status: ${response.status}`);
  } catch (error) {
    console.log(`✗ Failed:`, error.message);
  }
}

// Run tests
main().catch(console.error);