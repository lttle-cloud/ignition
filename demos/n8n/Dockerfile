FROM docker.n8n.io/n8nio/n8n:latest

USER node

RUN mkdir /home/node/lttle
COPY hook.js /home/node/lttle/hook.js

ENV EXTERNAL_HOOK_FILES=/home/node/lttle/hook.js