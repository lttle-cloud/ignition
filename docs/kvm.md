### [temp] kvm setup


1. Add user to the kvm group 
```bash
[ $(stat -c "%G" /dev/kvm) = kvm ] && sudo usermod -aG kvm ${USER} \
&& echo "Access granted."
```

2. Restart your shell to reload permissions

3. Check if kvm works
```bash
[ -r /dev/kvm ] && [ -w /dev/kvm ] && echo "OK" || echo "FAIL"
```