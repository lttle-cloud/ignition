[<p align="center" width="100%"><img src="./assets/lttle.gif"></p>](https://lttle.cloud)

# lttle.cloud | Ignition

With modern FaaS and serverless workloads, startup times can cause latencies that degrade user experience.

Ignition is a microVM manager built on KVM and designed to handle FaaS-like workloads with near-instant startup times. Inspired by [Unikraft Cloud](https://unikraft.cloud), Ignition aims to provide a cloud-agnostic platform for running networked applications with minimal overhead.

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

## State of the project

Ignition is in its early stages of development. We’re actively working on the core functionality and tuning performance to achieve our primary goals. That said, we already have a working prototype that showcases the snapshot-based approach and near-instant startup times. You can [try it out here](https://hello.lttle.cloud/).
