
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