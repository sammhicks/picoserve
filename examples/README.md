# Examples

## Embassy on Raspberry Pi Pico

| Example                                                                                            | Description                                                                                |
| -------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| [`hello_world`](embassy/hello_world/src/main.rs)                                                   | A minimal example showing how to set up a `Router`.                                        |
| [`hello_world_defmt`](embassy/hello_world_defmt/src/main.rs)                                       | A minimal example showing how to set up a `Router` using a debugger and defmt for logging. |
| [`set_led`](embassy/set_pico_w_led/src/main.rs)                                                    | Controlling the LED on the Raspberry Pi Pico via a web interface.                          |
| [`app_with_props`](embassy/app_with_props/src/main.rs)                                             | Passing data when building the App.                                                        |
| [`graceful_shutdown_using_future_array`](embassy/graceful_shutdown_using_future_array/src/main.rs) | Graceful shutdown using an array of `Future`s.                                             |
| [`graceful_shutdown_using_tasks`](embassy/graceful_shutdown_using_tasks/src/main.rs)               | Graceful shutdown using `embassy_executor` tasks.                                          |
| [`huge_requests`](embassy/huge_requests/src/main.rs)                                               | Extending the read timeout to support huge requests.                                       |
| [`various_states`](embassy/various_states/src/main.rs)                                             | Some different usages of application state.                                                |
| [`web_sockets`](embassy/web_sockets/src/main.rs)                                                   | A long-lived connection both sending and receiving WebSocket messages.                     |
## Tokio

| Example                                                                                    | Description                                                                                                  |
| ------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------ |
| [`hello_world`](hello_world/src/main.rs)                                                   | A minimal example showing how to set up a `Router`.                                                          |
| [`hello_world_single_thread`](hello_world_single_thread/src/main.rs)                       | A minimal example showing how to set up a `Router`, running on a single thread.                              |
| [`chunked_response`](chunked_response/src/main.rs)                                         | Usage of `Transfer-Encoding: chunked`.                                                                       |
| [`conditional_routing`](conditional_routing/src/main.rs)                                   | Runtime selection of two routers.                                                                            |
| [`custom_extractor`](custom_extractor/src/main.rs)                                         | How to extract custom data from a request.                                                                   |
| [`form`](form/src/main.rs)                                                                 | GET and POST Methods, and serving File.                                                                      |
| [`graceful_shutdown`](graceful_shutdown/src/main.rs)                                       | Gracefully shutting down the server.                                                                         |
| [`graceful_shutdown_server_sent_events`](graceful_shutdown_server_sent_events/src/main.rs) | Gracefully shutting down a server and its SSE connections.                                                   |
| [`graceful_shutdown_web_sockets`](graceful_shutdown_web_sockets/src/main.rs)               | Gracefully shutting down a server and its WS connections.                                                    |
| [`huge_requests`](huge_requests/src/main.rs)                                               | Extending the read timeout to support huge requests.                                                         |
| [`layers`](layers/src/main.rs)                                                             | Middleware example which logs how long a request took to be handled.                                         |
| [`nested_router`](nested_router/src/main.rs)                                               | Nesting `Router`s inside other `Router`s.                                                                    |
| [`path_parameters`](path_parameters/src/main.rs)                                           | Extracing data from path segments.                                                                           |
| [`query`](query/src/main.rs)                                                               | Extracting data from the url search.                                                                         |
| [`request_info`](request_info/src/main.rs)                                                 | A `MethodHandlerService` which reports information about the request.                                        |
| [`response_using_state`](response_using_state/src/main.rs)                                 | Returning a response which uses the State when writing itself to the socket.                                 |
| [`routing_fallback`](routing_fallback/src/main.rs)                                         | Providing a fallback `PathRouterService` when routing fails.                                                 |
| [`server_sent_events`](server_sent_events/src/main.rs)                                     | A long-lived connection generating Server-Sent Events with Keep-Alive messages.                              |
| [`state`](state/src/main.rs)                                                               | Stateful Applications.                                                                                       |
| [`state_local`](state_local/src/main.rs)                                                   | Stateful Applications with data coming from the connection.                                                  |
| [`state_multiple`](state_multiple/src/main.rs)                                             | How to have multiple differents States within a Router, allowing for separated stateful nested applications. |
| [`static_content`](static_content/src/main.rs)                                             | Serving static files such as HTML and CSS.                                                                   |
| [`web_sockets`](web_sockets/src/main.rs)                                                   | A long-lived connection both sending and receiving WebSocket messages.                                       |
