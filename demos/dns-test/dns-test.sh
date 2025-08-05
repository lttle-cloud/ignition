#!/bin/sh

echo "=== Testing Alpine DNS Fixes ==="
echo ""

echo "1. Testing DNS server connectivity:"
echo "Checking if nc is available:"
which nc || echo "nc not found"
echo "Testing with nslookup instead:"
nslookup google.com 10.1.0.1 2>&1
echo ""

echo "2. Testing lttle.local resolution:"
dig nginx-test.default.svc.lttle.local

echo ""
echo "3. Testing external DNS resolution:"
dig google.com

echo ""
echo "4. Checking /etc/resolv.conf:"
cat /etc/resolv.conf

echo ""
echo "5. Testing getent (musl libc resolver):"
getent hosts google.com || echo "getent failed with exit code: $?"

echo ""
echo "6. Checking ping version:"
ping -V 2>&1 || echo "No version info"
which ping

echo ""
echo "7. Testing ping with IP address:"
ping -c 1 142.250.201.174 2>&1

echo ""
echo "8. Testing ping with hostname (Alpine musl libc):"
ping -c 1 google.com 2>&1 || echo "ping failed with exit code: $?"

echo ""
echo "9. Testing wget (also uses musl libc):"
wget -O /dev/null -T 5 http://google.com 2>&1 | head -5

echo ""
echo "10. Checking if strace is available:"
which strace || echo "strace not found"

echo ""
echo "11. Testing with different resolver methods:"
echo "Testing with ping -4 (force IPv4):"
ping -4 -c 1 google.com 2>&1 || echo "Failed"

echo ""
echo "Testing if /etc/hosts is being checked:"
echo "142.250.201.174 google.com" >> /etc/hosts
ping -c 1 google.com 2>&1
sed -i '/google.com/d' /etc/hosts

echo ""
echo "12. Checking system configuration:"
echo "NSS configuration (/etc/nsswitch.conf):"
cat /etc/nsswitch.conf 2>/dev/null || echo "File not found"

echo ""
echo "13. Testing with busybox tools:"
echo "Using busybox nslookup:"
busybox nslookup google.com 2>&1

echo ""
echo "15. Testing DNS with tcpdump:"
echo "Testing direct DNS query with dig (verbose):"
dig @10.1.0.1 google.com +norecurse 2>&1

echo ""
echo "16. Testing if recursion is the issue:"
dig @10.1.0.1 google.com +recurse +short 2>&1

echo ""
echo "14. Testing capabilities:"
capsh --print 2>/dev/null | grep Current || echo "capsh not available"