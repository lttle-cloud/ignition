use util::{
    async_runtime::{runtime, time},
    result::Result,
};

async fn takeoff() {
    for i in 0..5 {
        println!("{}...", 5 - i);
        time::sleep(time::Duration::from_secs(1)).await;
    }

    println!("takeoff");

    time::sleep(time::Duration::from_secs(5)).await;
}

fn main() -> Result<()> {
    runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(takeoff());

    Ok(())
}
