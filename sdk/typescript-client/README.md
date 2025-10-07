# @lttle/client

A TypeScript client for the lttle.cloud API.

This client is auto-generated during the [Ignition](https://github.com/lttle-cloud/ignition) build process.

## Installation

```bash
npm install @lttle/client
```

## Usage

```typescript
import { Client } from '@lttle/client';

const client = new Client({
  baseUrl: 'https://eu.lttle.cloud',
  apiToken: 'your-api-token',
});

// List applications
const apps = await client.app.list();

// Get a specific machine
const [machine, status] = await client.machine.get('default', 'my-machine');

// Deploy a new app
await client.app.apply({
  'app.v1': {
    name: 'my-app',
    image: 'nginx:alpine',
    resources: {
      cpu: 1,
      memory: 512,
    },
  },
});
```

## Configuration

### ClientConfig

- `baseUrl`: The base URL of your Ignition server
- `token`: Your API authentication token
- `options` (optional): Additional options passed to the fetch implementation
- `fetch` (optional): Custom fetch implementation (defaults to global `fetch`)
- `WebSocket` (optional): Custom WebSocket implementation (defaults to global `WebSocket`)

### Using a custom fetch implementation

```typescript
import fetch from 'node-fetch';

const client = new Client({
  baseUrl: 'https://eu.lttle.cloud',
  apiToken: 'your-api-token',
  fetch,
});
```

### Using WebSocket for log streaming

The client supports WebSocket connections for real-time log streaming and command execution.

For Node.js environments, you can provide a custom WebSocket implementation:

```typescript
import WebSocket from 'ws';

const client = new ApiClient({
  baseUrl: 'https://eu.lttle.cloud',
  apiToken: 'your-api-token',
  WebSocket,
});
```

## Development

This client is auto-generated during the Ignition build process. Do not edit the generated files directly.

To regenerate the client:

```bash
cd /path/to/ignition
cargo build --features daemon
```

The generated TypeScript files will be in `sdk/typescript-client/src/`.