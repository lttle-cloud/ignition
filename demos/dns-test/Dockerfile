FROM alpine:3.19

# Install bind-tools for dig command
RUN apk add --no-cache bind-tools iputils-ping 
# Copy the test script
COPY dns-test.sh /dns-test.sh
RUN chmod +x /dns-test.sh

# Run the test script
CMD ["/dns-test.sh"]