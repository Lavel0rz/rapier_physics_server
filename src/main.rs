use futures_util::stream::SplitSink;
use futures_util::{sink::SinkExt, stream::StreamExt};
use rapier2d::prelude::*;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use warp::ws::{Message, WebSocket};
use warp::Filter;
use std::time::Instant;
use rusqlite::{Connection, Result as SqliteResult};
use std::process::Command;

struct Crane {
    min_x: f32,
    max_x: f32,
    y: f32,      // Y position where blocks will be dropped from
    position_x: f32, // Current x position of the crane
    direction: f32,  // Movement direction: 1.0 for right, -1.0 for left
    speed: f32,      // Speed at which the crane moves
}

impl Crane {
    fn new(min_x: f32, max_x: f32, y: f32) -> Self {
        Self {
            min_x,
            max_x,
            y,
            position_x: min_x,
            direction: 1.0,
            speed: 5.0, // Adjust the speed as needed
        }
    }

    // Move the crane left or right
    fn update_position(&mut self) {
        self.position_x += self.speed * self.direction;

        // Reverse direction when hitting the edges
        if self.position_x >= self.max_x {
            self.position_x = self.max_x;
            self.direction = -1.0;
        } else if self.position_x <= self.min_x {
            self.position_x = self.min_x;
            self.direction = 1.0;
        }
    }
}



#[derive(Serialize, Deserialize)]
struct CranePosition {
    position_x: f32,
    y: f32,
}

#[derive(Debug)]
pub enum MyError {
    SendError(tokio_tungstenite::tungstenite::Error),
    SerializeError(serde_json::Error),
    WarpError(warp::Error), // Include warp error type
}

#[derive(Debug, Clone)]
struct BlockType {
    block_type: String,
    width: f32,
    height: f32,
}

// Implementation for serde_json::Error
impl From<serde_json::Error> for MyError {
    fn from(err: serde_json::Error) -> MyError {
        MyError::SerializeError(err)
    }
}

// Implementation for tokio_tungstenite::tungstenite::Error
impl From<tokio_tungstenite::tungstenite::Error> for MyError {
    fn from(err: tokio_tungstenite::tungstenite::Error) -> MyError {
        MyError::SendError(err)
    }
}

// Implementation for warp::Error
impl From<warp::Error> for MyError {
    fn from(err: warp::Error) -> MyError {
        MyError::WarpError(err)
    }
}

// Optionally implement Display and Error traits for better error reporting
impl std::fmt::Display for MyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for MyError {}

#[derive(Serialize, Deserialize)]
struct PhysicsState {
    blocks: Vec<SerializableBlock>, // Use SerializableBlock for serialization
}

#[derive(Serialize, Deserialize)]
struct SerializableBlock {
    body_handle: usize, // Store an ID instead of the RigidBodyHandle
    position: (f32, f32),
    rotation: f32,
    block_type: String,
    width: f32,
    height: f32,
}

struct WsSender {
    id: usize,
    sender: Arc<Mutex<SplitSink<WebSocket, Message>>>, // Use Arc<Mutex<SplitSink<...>>>
}

static NEXT_ID: AtomicUsize = AtomicUsize::new(0); // Static counter for unique IDs

#[derive(Serialize, Deserialize)]
struct UpdatePosition {
    position: (f32, f32),
}

// Represent a block's state
#[derive(Clone, Debug)]
struct Block {
    body_handle: RigidBodyHandle,
    position: (f32, f32),
    rotation: f32,
    block_type: String,
    width: f32,
    height: f32,
}

// Shared state structure
#[derive(Default)]
struct SharedState {
    gotchi_position: (f32, f32),
    blocks: Vec<Block>,
    score: u32,
    last_block_time: Option<Instant>,
    game_over: bool,
}

fn calculate_score(blocks: &[Block]) -> u32 {
    if blocks.is_empty() {
        return 0;
    }
    
    // Find the highest block
    let highest_y = blocks.iter()
        .map(|block| block.position.1 - block.height / 2.0)
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(0.0);
    
    // Calculate score based on height (adjust multiplier as needed)
    let base_height = 900.0; // Height of the ground
    let height_difference = base_height - highest_y;
    (height_difference * 10.0) as u32 // 10 points per unit of height
}

// Add this struct to represent a high score
#[derive(Serialize, Deserialize)]
struct HighScore {
    player_name: String,
    score: u32,
}

// Add this function to initialize the database
fn init_db() -> SqliteResult<Connection> {
    let conn = Connection::open("highscores.db")?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS high_scores (
            id INTEGER PRIMARY KEY,
            player_name TEXT NOT NULL,
            score INTEGER NOT NULL
        )",
        [],
    )?;
    Ok(conn)
}

// Add this function to insert a high score
fn insert_high_score(conn: &Connection, player_name: &str, score: u32) -> SqliteResult<()> {
    conn.execute(
        "INSERT INTO high_scores (player_name, score) VALUES (?1, ?2)",
        [player_name, &score.to_string()],
    )?;
    Ok(())
}

// Add this function to get top 10 high scores
fn get_top_scores(conn: &Connection) -> SqliteResult<Vec<HighScore>> {
    let mut stmt = conn.prepare("SELECT player_name, score FROM high_scores ORDER BY score DESC LIMIT 10")?;
    let high_scores = stmt.query_map([], |row| {
        Ok(HighScore {
            player_name: row.get(0)?,
            score: row.get(1)?,
        })
    })?;
    high_scores.collect()
}

// Add this function to restart the server
fn restart_server() {
    println!("Restarting server...");
    if let Err(e) = Command::new(std::env::current_exe().unwrap()).spawn() {
        eprintln!("Failed to restart server: {:?}", e);
    } else {
        std::process::exit(0);
    }
}



#[tokio::main]


async fn main() {
    // Initialize shared state
    let crane = Arc::new(Mutex::new(Crane::new(100.0, 700.0, 100.0)));

    let shared_state = Arc::new(Mutex::new(SharedState {
        gotchi_position: (400.0, 100.0),
        blocks: Vec::new(),
        score: 0,
        last_block_time: None,
        game_over: false,
    }));

    // Physics simulation setup
    let gravity = vector![0.0, 490.0]; // Realistic gravity
    let integration_parameters = IntegrationParameters::default();

    // Wrap physics resources in Arc<Mutex<...>>
    let mut physics_pipeline = PhysicsPipeline::new();
    let island_manager = Arc::new(Mutex::new(IslandManager::new()));
    let broad_phase = Arc::new(Mutex::new(BroadPhase::new()));
    let narrow_phase = Arc::new(Mutex::new(NarrowPhase::new()));
    let impulse_joint_set = Arc::new(Mutex::new(ImpulseJointSet::new()));
    let multibody_joint_set = Arc::new(Mutex::new(MultibodyJointSet::new()));
    let ccd_solver = Arc::new(Mutex::new(CCDSolver::new()));
    let query_pipeline = Arc::new(Mutex::new(QueryPipeline::new()));

    // Wrap rigid body set and collider set in Arc<Mutex<...>> for sharing
    let rigid_body_set = Arc::new(Mutex::new(RigidBodySet::new()));
    let collider_set = Arc::new(Mutex::new(ColliderSet::new()));

    // Create static edges of the game area
    {
        let mut rigid_body_set = rigid_body_set.lock().await; // Use .await
        let restitution = 0.0; // Set restitution to 0 for non-bouncy edges

        let ground_body_handle = rigid_body_set.insert(
            RigidBodyBuilder::fixed()
                .translation(vector![400.0, 900.0])
                .build(),
        );
        collider_set.lock().await.insert_with_parent(
            ColliderBuilder::cuboid(400.0, 10.0)
                .restitution(restitution) // Set restitution for ground
                .build(),
            ground_body_handle,
            &mut rigid_body_set,
        );

        let top_body_handle = rigid_body_set.insert(
            RigidBodyBuilder::fixed()
                .translation(vector![400.0, 0.0])
                .build(),
        );
        collider_set.lock().await.insert_with_parent(
            ColliderBuilder::cuboid(400.0, 10.0)
                .restitution(restitution) // Set restitution for top
                .build(),
            top_body_handle,
            &mut rigid_body_set,
        );

        let left_body_handle = rigid_body_set.insert(
            RigidBodyBuilder::fixed()
                .translation(vector![0.0, 450.0])
                .build(),
        );
        collider_set.lock().await.insert_with_parent(
            ColliderBuilder::cuboid(10.0, 450.0)
                .restitution(restitution) // Set restitution for left
                .build(),
            left_body_handle,
            &mut rigid_body_set,
        );

        let right_body_handle = rigid_body_set.insert(
            RigidBodyBuilder::fixed()
                .translation(vector![800.0, 450.0])
                .build(),
        );
        collider_set.lock().await.insert_with_parent(
            ColliderBuilder::cuboid(10.0, 450.0)
                .restitution(restitution) // Set restitution for right
                .build(),
            right_body_handle,
            &mut rigid_body_set,
        );
    }

    // Start the physics simulation in a separate task
    let shared_state_clone = Arc::clone(&shared_state);
    let rigid_body_set_clone = Arc::clone(&rigid_body_set);
    let collider_set_clone = Arc::clone(&collider_set);
    let ws_senders = Arc::new(Mutex::new(Vec::new())); // Initialize WebSocket senders
    let ws_senders_clone = Arc::clone(&ws_senders);
    let crane_clone = Arc::clone(&crane);
    tokio::spawn(async move {
        let mut score_update_timer = 0;
        let mut final_score_sent = false;

        loop {
            //eprintln!("Starting physics update...");

            // Lock all required components
            let mut rigid_body_set = rigid_body_set_clone.lock().await;
            let mut collider_set = collider_set_clone.lock().await;
            let mut island_manager = island_manager.lock().await;
            let mut broad_phase = broad_phase.lock().await;
            let mut narrow_phase = narrow_phase.lock().await;
            let mut impulse_joint_set = impulse_joint_set.lock().await;
            let mut multibody_joint_set = multibody_joint_set.lock().await;
            let mut ccd_solver = ccd_solver.lock().await;
            let mut query_pipeline = query_pipeline.lock().await;

            // Step the physics simulation
            physics_pipeline.step(
                &gravity,
                &integration_parameters,
                &mut *island_manager,
                &mut *broad_phase,
                &mut *narrow_phase,
                &mut *rigid_body_set,
                &mut *collider_set,
                &mut *impulse_joint_set,
                &mut *multibody_joint_set,
                &mut *ccd_solver,
                Some(&mut *query_pipeline),
                &(),
                &(),
            );

            //eprintln!("Physics update completed.");

            // Update blocks logic
            {
                let mut state = shared_state_clone.lock().await; // Lock shared state
                                                                 // Update blocks based on their current state
                for block in &mut state.blocks {
                    let block_body = &rigid_body_set[block.body_handle]; // Accessing the body
                    block.position = (block_body.translation().x, block_body.translation().y);
                    block.rotation = block_body.rotation().angle();

                    // Log the new position for debugging
                    //eprintln!("Block ID: {:?} New Position: {:?}, Rotation: {:?}", block.body_handle, block.position, block.rotation);
                }
            }
                            // Inside the physics simulation loop
                {
                    let crane = crane_clone.lock().await;
                    let crane_position = CranePosition {
                        position_x: crane.position_x,
                        y: crane.y,
                    };

                    // Broadcast the crane's position
                    if let Err(e) = broadcast_crane_position(ws_senders.clone(), crane_position).await {
                        eprintln!("Failed to broadcast crane position: {:?}", e);
                    }
                }

            // Broadcast updated block positions
            {
                let state = shared_state_clone.lock().await; // Lock is only held for broadcast
                if let Err(e) = broadcast_block_updates(ws_senders.clone(), &state).await {
                    eprintln!("Failed to broadcast block updates: {:?}", e);
                } else {
                    // eprintln!("Successfully broadcasted block updates.");
                }
            }

            //eprintln!("Sleeping for ~60 FPS.");
            tokio::time::sleep(tokio::time::Duration::from_millis(16)).await; // For ~60 FPS

            // Update score every second (assuming 60 FPS)
            score_update_timer += 1;
            if score_update_timer >= 60 {
                score_update_timer = 0;
                let mut state = shared_state_clone.lock().await;
                
                // Only update score if the game is not over
                if !state.game_over {
                    state.score = calculate_score(&state.blocks);
                    
                    // Broadcast the updated score
                    if let Err(e) = broadcast_score(ws_senders.clone(), state.score, false).await {
                        eprintln!("Failed to broadcast score update: {:?}", e);
                    }
                    
                    // Check if it's time to calculate the final score
                    if let Some(last_time) = state.last_block_time {
                        if last_time.elapsed().as_secs() >= 10 && !final_score_sent && state.blocks.len() >= 10 {
                            state.score = calculate_score(&state.blocks);
                            state.last_block_time = None; // Reset the timer
                            state.game_over = true;
                            final_score_sent = true;

                            // Broadcast the final score
                            if let Err(e) = broadcast_score(ws_senders.clone(), state.score, true).await {
                                eprintln!("Failed to broadcast final score: {:?}", e);
                            }
                        } else if !final_score_sent && state.blocks.len() >= 10 {
                            // Send a message to indicate the stabilizing phase
                            let stabilizing_message = serde_json::json!({
                                "type": "stabilizing_phase",
                                "remaining_time": 10 - last_time.elapsed().as_secs()
                            });
                            if let Err(e) = broadcast_message(ws_senders.clone(), &stabilizing_message).await {
                                eprintln!("Failed to broadcast stabilizing phase message: {:?}", e);
                            }
                        }
                    }
                }
            }
        }
    });

    

            // Crane movement simulation task
            let crane_clone = Arc::clone(&crane);
            tokio::spawn(async move {
                loop {
                    {
                        let mut crane = crane_clone.lock().await;
                        crane.update_position();
                        // You can log the position here for debugging
                        // eprintln!("Crane position: {}", crane.position_x);
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(16)).await; // Adjust to control speed
                }
            });

            

    let ws_handler = {
        let shared_state = Arc::clone(&shared_state);
        let rigid_body_set_clone = Arc::clone(&rigid_body_set);
        let collider_set_clone = Arc::clone(&collider_set);
        let db_conn = Arc::new(Mutex::new(init_db().expect("Failed to initialize database")));

        warp::path("ws")
            .and(warp::ws())
            .map(move |ws: warp::ws::Ws| {
                let shared_state = Arc::clone(&shared_state);
                let rigid_body_set_clone = Arc::clone(&rigid_body_set_clone);
                let crane = Arc::clone(&crane);
                let collider_set_clone = Arc::clone(&collider_set_clone);
                let ws_senders_clone = Arc::clone(&ws_senders_clone);
                let db_conn = Arc::clone(&db_conn);

                ws.on_upgrade(move |websocket| {
                    handle_socket(
                        shared_state,
                        crane,
                        rigid_body_set_clone,
                        collider_set_clone,
                        websocket,
                        ws_senders_clone,
                        db_conn,
                    )
                })
            })
    };

    // CORS configuration
    let cors = warp::cors()
        .allow_any_origin() // Allows all origins. Modify as needed for production.
        .allow_headers(vec!["Content-Type"])
        .allow_methods(vec!["GET", "POST", "OPTIONS"]);

    // Combine the routes with CORS
    let routes = ws_handler.with(cors);

    // Start the warp server
    warp::serve(routes).run(([0, 0, 0, 0], 3030)).await;
}

async fn broadcast_crane_position(
    ws_senders: Arc<Mutex<Vec<WsSender>>>,
    crane_position: CranePosition,
) -> Result<(), MyError> {
    let serialized_position = serde_json::to_string(&crane_position)?;

    let senders = ws_senders.lock().await;
    for ws_sender in senders.iter() {
        let mut split_sender = ws_sender.sender.lock().await; // Lock the sender
        if let Err(e) = split_sender.send(Message::text(&serialized_position)).await {
            eprintln!(
                "Error sending crane position update to client ID {}: {:?}",
                ws_sender.id, e
            );
            return Err(MyError::from(e)); // Convert the error to MyError
        }
    }

    Ok(())
}
async fn broadcast_block_updates(
    ws_senders: Arc<Mutex<Vec<WsSender>>>,
    state: &SharedState,
) -> Result<(), MyError> {
    let serialized_state = serde_json::to_string(&PhysicsState {
        blocks: state
            .blocks
            .iter()
            .enumerate()
            .map(|(index, block)| SerializableBlock {
                body_handle: index,
                position: block.position,
                rotation: block.rotation,
                block_type: block.block_type.clone(),
                width: block.width,
                height: block.height,
            })
            .collect(),
    })?;

    //eprintln!("Broadcasting block updates to all clients: {}", serialized_state);

    let senders = ws_senders.lock().await;
    for ws_sender in senders.iter() {
        let mut split_sender = ws_sender.sender.lock().await; // Lock the sender
        if let Err(e) = split_sender.send(Message::text(&serialized_state)).await {
            eprintln!(
                "Error sending update message to client ID {}: {:?}",
                ws_sender.id, e
            );
            return Err(MyError::from(e)); // Convert the error to MyError
        } else {
            //eprintln!("Successfully sent block update to client ID {}", ws_sender.id);
        }
    }

    Ok(())
}

async fn handle_socket(
    shared_state: Arc<Mutex<SharedState>>,
    crane: Arc<Mutex<Crane>>, // Crane position
    rigid_body_set: Arc<Mutex<RigidBodySet>>,
    collider_set: Arc<Mutex<ColliderSet>>,
    websocket: WebSocket,
    ws_senders: Arc<Mutex<Vec<WsSender>>>, // Use Vec<WsSender> to match the WsSender struct
    db_conn: Arc<Mutex<Connection>>,
) {
    let (sender, mut receiver) = websocket.split();

    // Store the new sender
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let new_sender = WsSender {
        id,
        sender: Arc::new(Mutex::new(sender)),
    };

    {
        let mut senders = ws_senders.lock().await;
        senders.push(new_sender);
        eprintln!("New client connected with ID: {}", id);
    }

    // Define the block type dimensions
    let block_dimensions: [(u32, u32); 5] = [
        (80, 40),  // Block type 1
        (80, 80),  // Block type 2
        (40, 80),  // Block type 3
        (40, 120), // Block type 4
        (60, 200), // Block type 5
    ];

    // Block type sequence
    let block_sequence = vec![1, 1, 3, 2, 5, 4, 4, 3, 2, 1]; // This will repeat until we reach the limit
    let mut sequence_index = 0;

    // Send initial score to the new client only if block limit is reached
    let initial_score = {
        let state = shared_state.lock().await;
        if state.blocks.len() >= 10 {
            Some(state.score)
        } else {
            None
        }
    };
    if let Some(score) = initial_score {
        if let Err(e) = broadcast_score(Arc::clone(&ws_senders), score, false).await {
            eprintln!("Failed to send initial score: {:?}", e);
        }
    }

    // Handle incoming messages
    while let Some(result) = receiver.next().await {
        match result {
            Ok(msg) => {
                if msg.is_text() {
                    if let Ok(text) = msg.to_str() {
                        if text.contains("\"action\":\"spawn_block\"") {
                            {
                                let mut rigid_body_set = rigid_body_set.lock().await;
                                let mut collider_set = collider_set.lock().await;
                                let mut state = shared_state.lock().await;
                                let crane = crane.lock().await;

                                if state.blocks.len() < 10 {
                                    let block_type = block_sequence[sequence_index % block_sequence.len()];
                                    let (width, height) = block_dimensions[(block_type - 1) as usize];
                        
                                    // Use the current crane position instead of a random one
                                    let position_x = crane.position_x;
                                    let position = (position_x, crane.y);
                                    let rotation = 0.0;
                        
                                    // Create the rigid body and collider
                                    let body_handle = rigid_body_set.insert(
                                        RigidBodyBuilder::dynamic()
                                            .translation(vector![position.0, position.1])
                                            .linvel(vector![0.0, 0.0])
                                            .angular_damping(2.0)
                                            .linear_damping(0.8)
                                            .build(),
                                    );
                        
                                    collider_set.insert_with_parent(
                                        ColliderBuilder::cuboid((width as f32) / 2.0, (height as f32) / 2.0)
                                            .restitution(0.0) // No bounce
                                            .friction(0.9) // High friction to prevent sliding
                                            .mass(1000000.0)

                                            .build(),
                                        body_handle,
                                        &mut rigid_body_set,
                                    );
                        
                                    let new_block = Block {
                                        body_handle,
                                        position,
                                        rotation,
                                        block_type: format!("{}", block_type),
                                        width: width as f32,
                                        height: height as f32,
                                    };
                        
                                    state.blocks.push(new_block);
                        
                                    // If this is the 10th block, start the timer
                                    if state.blocks.len() == 10 {
                                        state.last_block_time = Some(Instant::now());
                                    }
                        
                                    // Update sequence index for the next block
                                    sequence_index += 1;
                        
                                    if let Err(e) = broadcast_block_updates(ws_senders.clone(), &state).await {
                                        eprintln!("Failed to broadcast block updates after spawning a block: {:?}", e);
                                    }
                        
                                    if let Some(ws_sender) = ws_senders.lock().await.iter().find(|s| s.id == id) {
                                        let mut split_sender = ws_sender.sender.lock().await;
                                        if let Err(e) = split_sender.send(Message::text("Block spawned")).await {
                                            // Handle error if needed
                                        }
                                    }
                                } else {
                                    if let Some(ws_sender) = ws_senders.lock().await.iter().find(|s| s.id == id) {
                                        let mut split_sender = ws_sender.sender.lock().await;
                                        if let Err(e) = split_sender.send(Message::text("Block limit reached")).await {
                                            // Handle error if needed
                                        }
                                    }
                                }
                            }
                        }
                         else if text.contains("\"action\":\"save_score\"") {
                            let state = shared_state.lock().await;
                            if state.game_over {
                                if let Ok(save_request) = serde_json::from_str::<SaveScoreRequest>(text) {
                                    let conn = db_conn.lock().await;
                                    if let Err(e) = insert_high_score(&conn, &save_request.player_name, state.score) {
                                        eprintln!("Failed to save high score: {:?}", e);
                                    } else {
                                        // Send confirmation to client
                                        if let Some(ws_sender) = ws_senders.lock().await.iter().find(|s| s.id == id) {
                                            let mut split_sender = ws_sender.sender.lock().await;
                                            if let Err(e) = split_sender.send(Message::text("Score saved successfully")).await {
                                                eprintln!("Error sending score save confirmation: {:?}", e);
                                            }
                                        }
                                        // Restart the server after saving the score
                                        restart_server();
                                    }
                                }
                            }
                        } else if text.contains("\"action\":\"get_high_scores\"") {
                            let conn = db_conn.lock().await;
                            match get_top_scores(&conn) {
                                Ok(high_scores) => {
                                    let serialized_scores = serde_json::to_string(&high_scores).unwrap();
                                    if let Some(ws_sender) = ws_senders.lock().await.iter().find(|s| s.id == id) {
                                        let mut split_sender = ws_sender.sender.lock().await;
                                        if let Err(e) = split_sender.send(Message::text(serialized_scores)).await {
                                            eprintln!("Error sending high scores: {:?}", e);
                                        }
                                    }
                                }
                                Err(e) => eprintln!("Failed to get high scores: {:?}", e),
                            }
                        } else if text.contains("\"action\":\"reset_game\"") {
                            restart_server();
                            }
                         else {
                            if let Ok(update) = serde_json::from_str::<UpdatePosition>(&text) {
                                let mut state = shared_state.lock().await;
                                state.gotchi_position = update.position;
                                if let Err(e) =
                                    broadcast_block_updates(ws_senders.clone(), &state).await
                                {
                                    eprintln!("Failed to broadcast position update: {:?}", e);
                                }
                            } else {
                                eprintln!(
                                    "Failed to deserialize UpdatePosition for client ID {}: {:?}",
                                    id, text
                                );
                            }
                        }
                    }
                } else if msg.is_close() {
                    eprintln!("Client ID {} disconnected.", id);
                    break;
                }
            }
            Err(e) => {
                eprintln!("WebSocket error for client ID {}: {:?}", id, e);
                break;
            }
        }
    }

    // Remove the sender from the list upon disconnection
    {
        let mut senders = ws_senders.lock().await;
        senders.retain(|s| s.id != id);
        eprintln!(
            "Removed client ID {} from sender list. Remaining clients: {}",
            id,
            senders.len()
        );
    }
}

async fn broadcast_score(
    ws_senders: Arc<Mutex<Vec<WsSender>>>,
    score: u32,
    is_final: bool,
) -> Result<(), MyError> {
    let score_update = serde_json::json!({
        "type": "score_update",
        "score": score,
        "is_final": is_final
    });
    let serialized_score = serde_json::to_string(&score_update)?;

    let senders = ws_senders.lock().await;
    for ws_sender in senders.iter() {
        let mut split_sender = ws_sender.sender.lock().await;
        if let Err(e) = split_sender.send(Message::text(&serialized_score)).await {
            eprintln!(
                "Error sending score update to client ID {}: {:?}",
                ws_sender.id, e
            );
            return Err(MyError::from(e));
        }
    }

    Ok(())
}

// Add this new function to broadcast messages
async fn broadcast_message(
    ws_senders: Arc<Mutex<Vec<WsSender>>>,
    message: &serde_json::Value,
) -> Result<(), MyError> {
    let serialized_message = serde_json::to_string(message)?;

    let senders = ws_senders.lock().await;
    for ws_sender in senders.iter() {
        let mut split_sender = ws_sender.sender.lock().await;
        if let Err(e) = split_sender.send(Message::text(&serialized_message)).await {
            eprintln!(
                "Error sending message to client ID {}: {:?}",
                ws_sender.id, e
            );
            return Err(MyError::from(e));
        }
    }

    Ok(())
}

// Add this struct to parse the save score request
#[derive(Deserialize)]
struct SaveScoreRequest {
    player_name: String,
}
















