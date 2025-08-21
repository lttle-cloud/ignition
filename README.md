<p align="center">
  <a href="https://lttle.cloud">
    <img src="./assets/lttle.gif" alt="lttle.cloud" />
  </a>
</p>

# lttle.cloud | Ignition

With modern FaaS and serverless workloads, startup times can cause latencies that degrade user experience.

Ignition is an all-in-one solution for this problem: an orchestrator, a microVM manager built on KVM, and a TCP/TLS proxy designed to handle FaaS-like workloads with near-instant startup times. It aims to provide a cloud-agnostic platform for running networked applications with minimal overhead.

Key goals:

- **Millisecond cold boot**: Ignition strives to boot a microVM and serve traffic in under 10 ms.
- **No vendor lock-in**: Ships as a single binary and can run on any cloud provider (that supports hardware acceleration) or on bare metal.
- **Focus on networked applications**: Optimized for network services—no runtime or protocol constraints. Run Node.js HTTP servers, Go TCP servers, and more, without code modifications.
- **Linux only**: Ignition exclusively runs Linux kernels. While microkernels have potential, we believe the time isn’t right just yet.
- **Simple to use**: Ignition works with your existing applications, and can natively run OCI images.

## How is it so fast?

Ignition uses a snapshot-based approach to eliminate the overhead of application initialization. Here’s how it works:

1. Deploy and Initialize: When you deploy your application, Ignition launches a microVM with your application as userspace and a modified Linux kernel. It waits until the application fully initializes.
2. Snapshot and Shutdown: Once initialization is complete, Ignition saves the microVM’s state (including the application and kernel) and shuts it down.

3. On-Demand Startup: When network traffic arrives for your application, Ignition instantly restores the microVM from the saved snapshot and forwards the connection. By skipping the entire initialization process, Ignition achieves near-instant startup times.

Depending on your configuration, Ignition can capture the snapshot:

- Right after the application’s userspace starts,
- When the application signals that it’s network-ready (e.g., after the first listen syscall),
- Or at a custom-defined trigger within the application.

The modified Linux kernel detects these triggers automatically, so you don’t have to alter your application’s code. As a result, you can enjoy rapid startup times without any extra development overhead.

## Installation

At the moment we only provide pre-built binaries for the CLI on macOS ARM and Linux x86_64.

To install the `lttle` CLI, you can run:

```sh
curl -fsSL https://raw.githubusercontent.com/lttle-cloud/ignition/refs/heads/master/get/lttle.sh | bash
```

If you're on a different platform, you can build it from source:

```sh
git clone https://github.com/lttle-cloud/ignition.git
cd ignition
cargo build --release --bin lttle
# the built binary will be at this path: target/release/lttle
# make sure to move it to a location in your $PATH
```

The CLI also comes with completions for your shell. To get further instructions on how to install them, run:

```bash
lttle completions --help
```

## After installation

After installing the CLI, you will have to authenticate with an `ignitiond` server. You can do this by running:

```bash
lttle login
```

If you applied for the early-access program, you will receive the instructions and credentials for one of our hosted regions. If you didn't apply, you can do so [on our website](https://lttle.cloud), or you can build and self-host the `ignitiond` daemon. There are no docs on how to do this yet; you're on your own (feel free to [reach out to us on discord](https://discord.gg/xhNGGrZQja) if you need help).


## State of the project

Ignition is under **active** development. We don't recommend using it in production yet. If you're interested in contributing, reach-out to us on [discord](https://discord.gg/xhNGGrZQja) to coordinate.

## Acknowledgements

Some ideas were initially inspired by [Unikraft Cloud](https://unikraft.cloud).
