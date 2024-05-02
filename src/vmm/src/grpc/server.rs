use self::vmmorchestrator::{
    vmm_service_server::VmmService as VmmServiceTrait, Language, RunVmmRequest,
};
use crate::grpc::client::agent::ExecuteRequest;
use crate::VmmErrors;
use crate::{core::vmm::VMM, grpc::client::WorkloadClient};
use std::ffi::{OsStr, OsString};
use std::str::FromStr;
use std::time::Duration;
use std::{
    convert::From,
    env::current_dir,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::{error, info};

type Result<T> = std::result::Result<Response<T>, tonic::Status>;

pub mod vmmorchestrator {
    tonic::include_proto!("vmmorchestrator");
}

pub mod agent {
    tonic::include_proto!("cloudlet.agent");
}

// Implement the From trait for VmmErrors into Status
impl From<VmmErrors> for Status {
    fn from(error: VmmErrors) -> Self {
        // You can create a custom Status variant based on the error
        match error {
            VmmErrors::VmmNew(_) => Status::internal("Error creating VMM"),
            VmmErrors::VmmConfigure(_) => Status::internal("Error configuring VMM"),
            VmmErrors::VmmRun(_) => Status::internal("Error running VMM"),
        }
    }
}

#[derive(Default)]
pub struct VmmService;

impl VmmService {
    pub fn get_kernel(&self, curr_dir: &OsStr) -> std::result::Result<PathBuf, VmmErrors> {
        // define kernel path
        let mut kernel_entire_path = curr_dir.to_owned();
        kernel_entire_path
            .push("/tools/kernel/linux-cloud-hypervisor/arch/x86/boot/compressed/vmlinux.bin");

        // Check if the kernel is on the system, else build it
        let kernel_exists = Path::new(&kernel_entire_path)
            .try_exists()
            .expect("Unable to read directory");

        if !kernel_exists {
            info!("Kernel not found, building kernel");
            // Execute the script using sh and capture output and error streams
            let output = Command::new("sh")
                .arg("./tools/kernel/mkkernel.sh")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .expect("Failed to execute the kernel build script");

            // Print output and error streams
            info!("Script output: {}", String::from_utf8_lossy(&output.stdout));
            error!("Script errors: {}", String::from_utf8_lossy(&output.stderr));
        };
        Ok(PathBuf::from(&kernel_entire_path))
    }

    pub fn get_initramfs(
        &self,
        language: String,
        curr_dir: &OsStr,
    ) -> std::result::Result<PathBuf, VmmErrors> {
        // define initramfs file placement
        let mut initramfs_entire_file_path = curr_dir.to_owned();
        initramfs_entire_file_path.push("/tools/rootfs/");
        let image = String::from_str(&format!("{}:alpine", language)).unwrap();
        initramfs_entire_file_path.push(language);
        initramfs_entire_file_path.push(".img");

        let rootfs_exists = Path::new(&initramfs_entire_file_path)
            .try_exists()
            .unwrap_or_else(|_| {
                panic!("Could not access folder {:?}", &initramfs_entire_file_path)
            });
        if !rootfs_exists {
            // build the agent
            let agent_file_name = self.build_agent(curr_dir).unwrap();
            // build initramfs
            info!("Building initramfs");
            // Execute the script using sh and capture output and error streams
            let output = Command::new("sh")
                .arg("./tools/rootfs/mkrootfs.sh")
                .arg(&image)
                .arg(&agent_file_name)
                .arg(&initramfs_entire_file_path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .expect("Failed to execute the initramfs build script");

            // Print output and error streams
            info!("Script output: {}", String::from_utf8_lossy(&output.stdout));
            error!("Script errors: {}", String::from_utf8_lossy(&output.stderr));
            info!("Initramfs successfully built.")
        }
        Ok(PathBuf::from(&initramfs_entire_file_path))
    }

    pub fn build_agent(&self, curr_dir: &OsStr) -> std::result::Result<OsString, ()> {
        // check if agent binary exists
        let mut agent_file_name = curr_dir.to_owned();
        agent_file_name.push("/target/x86_64-unknown-linux-musl/release/agent");

        // if agent hasn't been build, build it
        let agent_exists = Path::new(&agent_file_name)
            .try_exists()
            .unwrap_or_else(|_| panic!("Could not access folder {:?}", &agent_file_name));
        if !agent_exists {
            //build agent
            info!("Building agent binary");
            // Execute the script using sh and capture output and error streams
            let output = Command::new("cargo")
                .arg("build")
                .arg("--release")
                .arg("--bin")
                .arg("agent")
                .arg("--target=x86_64-unknown-linux-musl")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .expect("Failed to build the agent");

            // Print output and error streams
            info!("Script output: {}", String::from_utf8_lossy(&output.stdout));
            error!("Script errors: {}", String::from_utf8_lossy(&output.stderr));
            info!("Agent binary successfully built.")
        }
        Ok(agent_file_name)
    }

    pub fn get_agent_request(&self, vmm_request: RunVmmRequest) -> ExecuteRequest {
        // Send the grpc request to start the agent
        let agent_request = ExecuteRequest {
            workload_name: vmm_request.workload_name,
            language: match vmm_request.language {
                0 => "rust".to_string(),
                1 => "python".to_string(),
                2 => "node".to_string(),
                _ => unreachable!("Invalid language"),
            },
            action: 2, // Prepare and run
            code: vmm_request.code,
            config_str: "[build]\nrelease = true".to_string(),
        };
        agent_request
    }
}

#[tonic::async_trait]
impl VmmServiceTrait for VmmService {
    type RunStream =
        ReceiverStream<std::result::Result<vmmorchestrator::ExecuteResponse, tonic::Status>>;

    async fn run(&self, request: Request<RunVmmRequest>) -> Result<Self::RunStream> {
        let (tx, rx) = tokio::sync::mpsc::channel(4);

        const HOST_IP: Ipv4Addr = Ipv4Addr::new(172, 29, 0, 1);
        const HOST_NETMASK: Ipv4Addr = Ipv4Addr::new(255, 255, 0, 0);
        const GUEST_IP: Ipv4Addr = Ipv4Addr::new(172, 29, 0, 2);

        // get current directory
        let curr_dir = current_dir().expect("Need to be able to access current directory path.");

        let kernel_path = self.get_kernel(curr_dir.as_os_str()).unwrap();

        // get request with the language
        let vmm_request = request.into_inner();
        let language: Language =
            Language::from_i32(vmm_request.language).expect("Unknown language");

        let initramfs_path = self.get_initramfs(language.as_str_name().to_lowercase(), curr_dir.as_os_str()).unwrap();

        let mut vmm = VMM::new(HOST_IP, HOST_NETMASK, GUEST_IP).map_err(VmmErrors::VmmNew)?;

        // Configure the VMM parameters might need to be calculated rather than hardcoded
        vmm.configure(1, 4000, kernel_path, &Some(initramfs_path))
            .map_err(VmmErrors::VmmConfigure)?;

        // Run the VMM in a separate task
        tokio::spawn(async move {
            info!("Running VMM");
            if let Err(err) = vmm.run().map_err(VmmErrors::VmmRun) {
                error!("Error running VMM: {:?}", err);
            }
        });

        // run the grpc client
        let grpc_client = tokio::spawn(async move {
            // Wait 2 seconds
            tokio::time::sleep(Duration::from_secs(2)).await;
            println!("Connecting to Agent service");

            WorkloadClient::new(GUEST_IP, 50051).await
        })
        .await
        .unwrap();

        let agent_request = self.get_agent_request(vmm_request);

        match grpc_client {
            Ok(mut client) => {
                info!("Successfully connected to Agent service");

                // Start the execution
                let mut response_stream = client.execute(agent_request).await?;

                // Process each message as it arrives
                while let Some(response) = response_stream.message().await? {
                    let vmm_response = vmmorchestrator::ExecuteResponse {
                        stdout: response.stdout,
                        stderr: response.stderr,
                        exit_code: response.exit_code,
                    };
                    tx.send(Ok(vmm_response)).await.unwrap();
                }
            }
            Err(e) => {
                error!("ERROR {:?}", e);
            }
        }

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}
