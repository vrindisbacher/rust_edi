use queue::queue_service_server::{QueueService, QueueServiceServer};
use queue::{NewQueueRequest, NewQueueResponse};
use tonic::{Request, Response, Status};

use crate::rqs::rqs_types::RQSEvent;
use crate::GLOBAL_DATA;

use self::queue::{DeleteQueueRequest, DeleteQueueResponse};

pub mod queue;

#[derive(Debug, Default)]
pub struct Queue;

impl Queue {
    pub fn new_queue_server() -> QueueServiceServer<Queue> {
        QueueServiceServer::new(Queue::default())
    }
}

#[tonic::async_trait]
impl QueueService for Queue {
    async fn new_queue(
        &self,
        request: Request<NewQueueRequest>,
    ) -> Result<Response<NewQueueResponse>, Status> {
        let inner = request.into_inner();
        let queue_id = inner.queue_id;
        let visibility_timeout = inner.visibility_timeout;
        let response = match GLOBAL_DATA
            .lock()
            .await
            .handle_event(RQSEvent::QueueCreated { queue_id, visibility_timeout })
            .await
        {
            Ok(_) => NewQueueResponse {
                success: true,
                data: "Successfully created queue".to_string(),
            },
            Err(e) => NewQueueResponse {
                success: false,
                data: format!("Failed to create queue. Failed with error: {e}"),
            },
        };
        Ok(Response::new(response))
    }

    async fn delete_queue(
        &self,
        request: Request<DeleteQueueRequest>,
    ) -> Result<Response<DeleteQueueResponse>, Status> {
        let queue_id = request.into_inner().queue_id;
        let response = match GLOBAL_DATA
            .lock()
            .await
            .handle_event(RQSEvent::QueueDeleted { queue_id })
            .await
        {
            Ok(_) => DeleteQueueResponse {
                success: true,
                data: "Successfully deleted queue".to_string(),
            },
            Err(e) => DeleteQueueResponse {
                success: true,
                data: format!("Failed to delete queue. Failed with error: {e}"),
            },
        };
        Ok(Response::new(response))
    }
}

#[cfg(test)]
mod queue_client_server_test {
    use std::time::Duration;

    use crate::rqs::{EVENT_LOG, LOG_ROOT, RQS};
    use crate::{
        message::Message,
        queue::{
            queue::{queue_service_client::QueueServiceClient, NewQueueRequest},
            Queue,
        },
        GLOBAL_DATA,
    };
    use serial_test::serial;
    use tonic::transport::Server;

    use super::queue::DeleteQueueRequest;

    async fn start() {
        delete_event_log();
        let mut rqs = GLOBAL_DATA.lock().await;
        rqs.clear();
        rqs.revive_from_log().await;
        spawn_server().await;
    }

    async fn spawn_server() {
        // totally hacky way of starting up the server
        tokio::spawn(async {
            let server_addr = "127.0.0.1:8080".parse().unwrap();
            Server::builder()
                .add_service(tonic_web::enable(Message::new_message_server()))
                .add_service(tonic_web::enable(Queue::new_queue_server()))
                .serve(server_addr)
                .await
                .unwrap()
        });
        tokio::time::sleep(Duration::from_secs(3)).await;
    }

    fn delete_event_log() {
        let _ = std::fs::remove_file(format!("{LOG_ROOT}{EVENT_LOG}"));
    }

    #[tokio::test]
    #[serial]
    async fn test_create_queue_request() {
        start().await;

        let client_addr = "http://127.0.0.1:8080";
        let mut client = QueueServiceClient::connect(client_addr)
            .await
            .expect("Could not create client");
        let request = NewQueueRequest {
            queue_id: "queue_1".to_string(),
            visibility_timeout: 5,
        };
        client
            .new_queue(request)
            .await
            .expect("Failed to create queue request");

        let rqs = GLOBAL_DATA.lock().await;
        let queues = rqs
            .get_queues()
            .iter()
            .map(|x| x.get_name())
            .collect::<Vec<&String>>();
        assert_eq!(queues, vec!["queue_1"]);
    }

    #[tokio::test]
    #[serial]
    async fn test_create_and_delete_queue_request() {
        start().await;

        let client_addr = "http://127.0.0.1:8080";
        let mut client = QueueServiceClient::connect(client_addr)
            .await
            .expect("Could not create client");
        let request = NewQueueRequest {
            queue_id: "queue_1".to_string(),
            visibility_timeout: 5,
        };
        client
            .new_queue(request)
            .await
            .expect("Failed to create queue request");

        let request = DeleteQueueRequest {
            queue_id: "queue_1".to_string(),
        };
        client
            .delete_queue(request)
            .await
            .expect("Failed to delete queue");

        let rqs = GLOBAL_DATA.lock().await;
        let queues = rqs
            .get_queues()
            .iter()
            .map(|x| x.get_name())
            .collect::<Vec<&String>>();
        assert_eq!(queues, vec![] as Vec<&String>);
    }

    #[tokio::test]
    #[serial]
    async fn test_multiple_concurrent_queue_requests() {
        start().await;
        let client_addr = "http://127.0.0.1:8080";
        let mut client = QueueServiceClient::connect(client_addr)
            .await
            .expect("Could not create client");
        let request1 = NewQueueRequest {
            queue_id: "queue_1".to_string(),
            visibility_timeout: 5,
        };
        let request2 = NewQueueRequest {
            queue_id: "queue_2".to_string(),
            visibility_timeout: 5,
        };
        futures::future::join_all([
            client.clone().new_queue(request1),
            client.new_queue(request2),
        ])
        .await;

        // should have two queues
        let rqs = GLOBAL_DATA.lock().await;
        let queues = rqs
            .get_queues()
            .iter()
            .map(|x| x.get_name())
            .collect::<Vec<&String>>();
        assert_eq!(queues, vec!["queue_1", "queue_2"]);

        // should also be able to revive the queues
        let mut rqs = RQS::new();
        rqs.revive_from_log().await;
        let queues = rqs
            .get_queues()
            .iter()
            .map(|x| x.get_name())
            .collect::<Vec<&String>>();
        assert_eq!(queues, vec!["queue_1", "queue_2"]);
    }
}
