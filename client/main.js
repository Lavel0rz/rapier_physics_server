const config = {
  type: Phaser.AUTO,
  width: 800,
  height: 900,
  scene: {
      preload: preload,
      create: create,
      update: update
  }
};
let finalScore = null;
let finalScoreText;
let lastTime = 0; // Track the last frame time
let frameCount = 0; // Count frames for FPS calculation
let fpsText; // Text object for FPS
let blockGraphicsArray = []; // Array to store graphics objects for each block
const game = new Phaser.Game(config);
let gotchi; // Player sprite
let blocks = []; // Array to hold blocks with positions and sizes
let canSpawnBlock = true; // Debounce flag
let graphics; // Declare graphics variable
let socket; // WebSocket connection
let crane; // Declare crane variabl
let scoreText; // Text object for score display
let saveScoreButton;
let stabilizingText;
let stabilizingTimer;

function preload() {
  // Preload images
  this.load.image('gotchi', 'gotchi.png'); // Path to gotchi image
  this.load.image('block', 'block.png'); // Path to block image
  this.load.image('crane', 'crane.png'); // Path to crane image
}


function create() {

  finalScoreText = this.add.text(400, 450, '', { fontSize: '32px', fill: '#ffffff' });
  finalScoreText.setOrigin(0.5);
  finalScoreText.setVisible(false);
 
  fpsText = this.add.text(10, 10, 'FPS: 0', { fontSize: '16px', fill: '#ffffff' });
  crane = this.add.sprite(100, 700, 'crane').setScale(0.36); // Adjust the scale as needed

  // Create the gotchi using the gotchi image
  gotchi = this.add.sprite(400, 100, 'gotchi').setScale(0.1);

  // Enable keyboard input
  this.cursors = this.input.keyboard.createCursorKeys();
  this.spaceKey = this.input.keyboard.addKey(Phaser.Input.Keyboard.KeyCodes.SPACE); // Add spacebar key

  // Add listener for space key press
  this.input.keyboard.on('keydown-SPACE', spawnBlock, this);

  // Initialize graphics
  graphics = this.add.graphics();

  // Establish WebSocket connection
  socket = new WebSocket('ws://127.0.0.1:3030/ws');

  // Handle WebSocket events
  socket.onopen = function(event) {
      console.log('WebSocket connection opened:', event);
  };
  socket.onmessage = function(event) {
      try {
          const data = JSON.parse(event.data);
  
          if (data.position_x !== undefined && data.y !== undefined) {
              updateCranePosition(data);
          } else if (data.blocks) {
              handleServerUpdate(data);
          } else if (data.type === "score_update") {
              if (data.is_final) {
                  finalScore = data.score;
                  displayFinalScore();
                  clearStabilizingTimer();
              } else {
                  // Update the current score display
                  //
              }
          } else if (data.type === "stabilizing_phase") {
              startStabilizingTimer(data.remaining_time);
          } else if (Array.isArray(data)) {
              // Assume this is the high scores list
              displayHighScores(data);
          } else if (typeof data === 'string') {
              console.log('Server message:', data);
          }
      } catch (error) {
          console.error('Error parsing message from server:', error);
          console.log('Received non-JSON message:', event.data);
      }
  };
  

  socket.onclose = function(event) {
      console.log('WebSocket connection closed:', event);
      if (!event.wasClean) {

          location.reload();
      }
  };

  socket.onerror = function(event) {
      console.error('WebSocket error:', event);
  };

  // Create the save score button but keep it hidden initially
  saveScoreButton = this.add.text(400, 500, 'Save Score', { fontSize: '24px', fill: '#ffffff' })
      .setOrigin(0.5)
      .setInteractive()
      .on('pointerdown', () => {
          const playerName = prompt("Enter your name:");
          if (playerName) {
              saveScore(playerName, finalScore);
          }
      });
  saveScoreButton.setVisible(false);

  // Add a button to view high scores
  const viewHighScoresButton = this.add.text(650, 80, 'High Scores', { fontSize: '16px', fill: '#ffffff' })
      .setInteractive()
      .on('pointerdown', () => {
          requestHighScores();
      });

  // Add a reset game button
  const resetGameButton = this.add.text(650, 110, 'Reset Game', { fontSize: '16px', fill: '#ffffff' })
      .setInteractive()
      .on('pointerdown', () => {
          resetGame();
      });

  // Create the stabilizing phase text
  stabilizingText = this.add.text(400, 450, '', { fontSize: '32px', fill: '#ffffff' });
  stabilizingText.setOrigin(0.5);
  stabilizingText.setVisible(false);
}

function displayFinalScore() {
  if (finalScore !== null) {
      finalScoreText.setText(`Final Score: ${finalScore}`);
      finalScoreText.setVisible(true);
      saveScoreButton.setVisible(true);
      console.log("Final score:", finalScore);
      clearStabilizingTimer(); // Ensure the stabilizing text is cleared
  }
}

function updateCranePosition(data) {
  // Read position_x and y from the data object
  const craneX = data.position_x; // Get x position
  const craneY = data.y; // Get y position

  // Update crane's position
  crane.x = craneX; // Update crane x position
  crane.y = craneY; // Update crane y position
}

function drawBlocks() {
  // Clear the overall graphics layer first
  graphics.clear(); // Optionally keep if you are still using graphics elsewhere

  // Draw each block with its size and rotation
  blocks.forEach((block, index) => {
      const { x, y, width, height, angle, type } = block; // Destructure properties from block

      // Create a new image for each block
      let blockImage;
      
      // Check if the block already has an associated image
      if (blockGraphicsArray[index]) {
          blockImage = blockGraphicsArray[index]; // Reuse existing image object
          blockImage.setPosition(x, y); // Update position
          blockImage.setRotation(angle); // Update rotation
      } else {
          // Create a new image and store it
          blockImage = this.add.image(x, y, 'block'); // Use the block image
          blockGraphicsArray[index] = blockImage; // Store the new image object
      }

      // Scale the image based on block type (optional)
      blockImage.setDisplaySize(width, height); // Set width and height for the image
      blockImage.setOrigin(0.5, 0.5); // Set origin to center for rotation
  });
}

// Handle incoming updates from the server
function handleServerUpdate(data) {
    if (data.type === "score_update") {
        updateScoreDisplay(data.score);
        if (data.is_final) {
            finalScore = data.score;
            displayFinalScore();
        }
    }
    // Update the block positions and angles from the server
    if (data.blocks && data.blocks.length > 0) {
        blocks = data.blocks.map((block) => {
            const position = block.position; // Correctly accessing the position array
            let angle = block.rotation;      // Correctly accessing the rotation
            const blockType = block.block_type; // Correctly accessing the block type
        
            // Validate angle; set default to 0 if invalid
            if (angle === undefined || angle === null || isNaN(angle)) {
                console.error("Invalid angle value:", angle);
                angle = 0; // Default to 0 if angle is invalid
            }

            return {
                x: position[0], // Position X
                y: position[1], // Position Y
                width: block.width, // Access the width directly
                height: block.height, // Access the height directly
                angle: angle, // Convert angle from degrees to radians
                type: blockType // Use the block type for color mapping
            };
        });
        //console.log("Updated blocks:", blocks); // Log updated block states
    }
}
function updateScoreDisplay(score) {
    if (scoreText) {
        scoreText.setText(`Score: ${score}`);
    }
}
// Function to determine width based on block type
function getBlockWidth(blockType) {
  switch (blockType) {
      case "rect_horizontal":
          return 80;
      case "square":
          return 40;
      case "rect_vertical":
          return 20;
      case "long_vertical_3":
          return 40;
      case "long_vertical_5":
          return 40;
      default:
          return 20; // Default size
  }
}

// Function to determine height based on block type
function getBlockHeight(blockType) {
  switch (blockType) {
      case "rect_horizontal":
          return 20;
      case "square":
          return 40;
      case "rect_vertical":
          return 80;
      case "long_vertical_3":
          return 120;
      case "long_vertical_5":
          return 200;
      default:
          return 20; // Default size
  }
}

// Send position update via WebSocket
function sendPositionUpdate() {
  const position = { position: [gotchi.x, gotchi.y] };
  socket.send(JSON.stringify(position)); // Send the position update over WebSocket
}

// Request server to spawn a new block
function spawnBlock() {
  if (!canSpawnBlock) return; // Prevent spawning if already spawning a block

  canSpawnBlock = false; // Set the flag to prevent further spawns

  // Request the server to spawn a new block
  socket.send(JSON.stringify({ action: 'spawn_block' }));

  setTimeout(() => {
      canSpawnBlock = true; // Reset the flag after the spawn process is complete
  }, 1000); // Prevent rapid spawns

}

// Interpolation and prediction setup
let lastBlockUpdate = Date.now();

function update() {
  // Move the gotchi based on keyboard input
  let moved = false;

  if (this.cursors.left.isDown) {
      gotchi.x -= 5; // Move left
      moved = true;
  } else if (this.cursors.right.isDown) {
      gotchi.x += 5; // Move right
      moved = true;
  }

  if (this.cursors.up.isDown) {
      gotchi.y -= 5; // Move up
      moved = true;
  } else if (this.cursors.down.isDown) {
      gotchi.y += 5; // Move down
      moved = true;
  }

  // Check if the position has changed
  if (moved) {
      // Send position update to the server
      sendPositionUpdate();
  }

  // Calculate FPS
  const now = performance.now(); // Get the current time
  frameCount++; // Increment frame count

  if (now - lastTime >= 1000) { // Calculate FPS every second
      fpsText.setText(`FPS: ${frameCount}`); // Update the FPS display
      frameCount = 0; // Reset frame count
      lastTime = now; // Update last time
  }

  // Calculate the time since the last block update
  const deltaTime = (now - lastBlockUpdate) / 1000; // Convert to seconds

  // Interpolate block positions based on the last known velocities
  interpolateBlocks(deltaTime);

  // Draw blocks in the scene
  drawBlocks.call(this); // Use 'call' to ensure 'this' refers to the scene
}

// Interpolate blocks to make their movement smooth
function interpolateBlocks(deltaTime) {
  blocks.forEach((block) => {
      // Use some logic to calculate the predicted position
      block.x += block.velocityX * deltaTime; // Update the x position based on velocity
      block.y += block.velocityY * deltaTime; // Update the y position based on velocity
  });
}

// Handle incoming updates from the server
function handleServerUpdate(data) {
  // Update the block positions and angles from the server
  if (data.blocks && data.blocks.length > 0) {
      blocks = data.blocks.map((block) => {
          const position = block.position; // Correctly accessing the position array
          let angle = block.rotation;      // Correctly accessing the rotation
          const blockType = block.block_type; // Correctly accessing the block type

          // Set default velocity if not provided
          const velocityX = block.velocityX || 0;
          const velocityY = block.velocityY || 0;

          // Validate angle; set default to 0 if invalid
          if (angle === undefined || angle === null || isNaN(angle)) {
              console.error("Invalid angle value:", angle);
              angle = 0; // Default to 0 if angle is invalid
          }

          return {
              x: position[0], // Position X
              y: position[1], // Position Y
              width: block.width, // Access the width directly
              height: block.height, // Access the height directly
              angle: angle, // Convert angle from degrees to radians
              type: blockType, // Use the block type for color mapping
              velocityX: velocityX, // Store the x velocity
              velocityY: velocityY  // Store the y velocity
          };
      });
      //console.log("Updated blocks:", blocks); // Log updated block states
      lastBlockUpdate = Date.now(); // Update the last received block time
  }
}

function saveScore(playerName) {
    const saveScoreMessage = {
        action: 'save_score',
        player_name: playerName
    };
    socket.send(JSON.stringify(saveScoreMessage));
    
    // Add a listener for the server's response
    socket.onmessage = function(event) {
        if (event.data === "Score saved successfully") {
            alert("Score saved successfully. The game will restart.");
            location.reload(); // Reload the page to restart the game
        }
    };
}

function requestHighScores() {
    const requestHighScoresMessage = {
        action: 'get_high_scores'
    };
    socket.send(JSON.stringify(requestHighScoresMessage));
}

function displayHighScores(highScores) {
    // Create a modal or update an existing element to show high scores
    const highScoreText = highScores.map((score, index) => 
        `${index + 1}. ${score.player_name}: ${score.score}`
    ).join('\n');

    alert('High Scores:\n\n' + highScoreText);
}

function startStabilizingTimer(remainingTime) {
    clearStabilizingTimer(); // Clear any existing timer

    stabilizingText.setText(`STABILIZING PHASE: ${remainingTime}`);
    stabilizingText.setVisible(true);

    stabilizingTimer = setInterval(() => {
        remainingTime--;
        if (remainingTime > 0) {
            stabilizingText.setText(`STABILIZING PHASE: ${remainingTime}`);
        } else {
            clearStabilizingTimer();
        }
    }, 1000);
}

function clearStabilizingTimer() {
    if (stabilizingTimer) {
        clearInterval(stabilizingTimer);
        stabilizingTimer = null;
    }
    stabilizingText.setVisible(false);
}

function resetGame() {
    if (confirm("Are you sure you want to reset the game? This will clear all blocks and reset the score.")) {
        // Send a reset game request to the server
        socket.send(JSON.stringify({ action: 'reset_game' }));
        
        // Clear local game state
        blocks = [];
        finalScore = null;
        
        // Update UI
        updateScoreDisplay();
        finalScoreText.setVisible(false);
        saveScoreButton.setVisible(false);
        clearStabilizingTimer();
        
        // Optionally, you can add more reset logic here if needed
        
        console.log("Game reset requested");
    }
}

