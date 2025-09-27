
## [temp] certs

```bash
# generate self-signed default certs
mkdir -p certs
cd certs
openssl req  -nodes -new -x509 -keyout server.key -out server.cert
```

```bash
# generate registry cert
cd registry-stack
mkdir -p certs
cd certs
openssl req -x509 -newkey rsa:4096 -days 3650 -nodes \
  -subj "/CN=lttle.cloud" \
  -keyout token-signing.key -out token-root.pem
```


```bash
# generate builders CA & cert
cd build-stack
mkdir -p certs
cd certs

# generate CA
step certificate create "Lttle Build Root CA" \
  ca.pem ca.key \
  --profile root-ca --no-password --insecure

# generate cert
sudo mkdir -p /etc/buildkit/tls
step certificate create builder.tld \
  /etc/buildkit/tls/server.pem /etc/buildkit/tls/server.key \
  --profile leaf --not-after=87600h \
  --san builder.tld \
  --kty EC --curve P-256 \
  --no-password --insecure \
  --ca /etc/buildkit/tls/ca.pem --ca-key /etc/buildkit/tls/ca.key
```