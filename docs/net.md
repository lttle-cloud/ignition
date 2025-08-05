## [temp] network setup

```bash
# enable ip forwarding
sudo sysctl -w net.ipv4.ip_forward=1
echo "net.ipv4.ip_forward = 1" | sudo tee /etc/sysctl.d/99-sysctl.conf

# Create a `nat` table if it doesn’t exist
sudo nft add table ip nat

# Postrouting chain: masquerade 10.0.0.0/16 → enp6s0
sudo nft 'add chain ip nat POSTROUTING { type nat hook postrouting priority 100; }'
sudo nft 'add rule ip nat POSTROUTING ip saddr 10.0.0.0/16 oifname "enp6s0" masquerade'

# Create a `filter` table
sudo nft add table ip filter

# Forward chain: DROP by default
sudo nft 'add chain ip filter FORWARD { type filter hook forward priority 0; policy drop; }'

# Create bridge
sudo ip link add ltbr0 type bridge

# Adds host → vm routes
sudo ip addr add 10.0.0.1/16 dev ltbr0

# Bring it up
sudo ip link set ltbr0 up

# 4.0 Allow host → VM traffic
sudo nft 'add rule ip filter FORWARD iif "enp6s0" oif "ltbr0" accept'

# 4.1 Allow VM→Internet (ltbr0 → enp6s0)
sudo nft 'add rule ip filter FORWARD iif "ltbr0" oif "enp6s0" accept'

# 4.2 Allow replies (enp6s0 → ltbr0) for established connections
sudo nft 'add rule ip filter FORWARD iif "enp6s0" oif "ltbr0" ct state related,established accept'

# 4.3 Drop inter‐VM forwarding (ltbr0 → ltbr0)
sudo nft 'add rule ip filter FORWARD iif "ltbr0" oif "ltbr0" drop'

# 4.4 Allow host ↔ bridge via loopback
sudo nft 'add rule ip filter FORWARD iif "lo"    oif "ltbr0" accept'
sudo nft 'add rule ip filter FORWARD iif "ltbr0" oif "lo"    accept'

# add route to local svc
sudo ip route add local 10.1.0.0/16 dev lo

# (Optional) tighten ARP on the bridge so only ltbr0’s 10.0.0.1 responds:
# echo 1 | sudo tee /proc/sys/net/ipv4/conf/ltbr0/arp_ignore
# echo 2 | sudo tee /proc/sys/net/ipv4/conf/ltbr0/arp_announce
```