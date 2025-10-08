const counterOutput = document.getElementById("counterOutput");

const echoInput = document.getElementById("echoInput");
const echoOutput = document.getElementById("echoOutput");

const shutdownButton = document.getElementById("shutdownButton");
const shutdown = document.getElementById("shutdown");
const shutdownMessage = document.getElementById("shutdownMessage");
const disconnectionMessage = document.getElementById("disconnectionMessage");

shutdownButton.addEventListener("click", async () => {
    const message = await (await fetch("shutdown", {
        method: "POST",
    })).text();

    shutdownMessage.innerText = message;
});

const currentPath = window.location.pathname;

let websocketUri = (window.location.protocol === "https:") ? "wss:" : "ws:";
websocketUri += "//" + window.location.host;
websocketUri += currentPath.slice(0, currentPath.lastIndexOf("/") + 1) + "ws";

const ws = new WebSocket(websocketUri);

let wsIsConnected = false;

ws.addEventListener("open", () => {
    wsIsConnected = true;
})

ws.addEventListener("close", (ev) => {
    ws.close();

    disconnectionMessage.innerText = `Disconnected: ${ev.reason} (${ev.code})`;
    wsIsConnected = false;

    echoInput.remove();
});

ws.addEventListener("message", (ev) => {
    const message = JSON.parse(ev.data);

    switch (message.type) {
        case "Count":
            counterOutput.innerText = message.value;

            break;
        case "Echo":
            echoOutput.innerText = message.payload;

            break;
        default:
            console.error("Unknown message: ", message);
    }
});

echoInput.addEventListener("input", () => {
    if (wsIsConnected) {
        ws.send(echoInput.value);
    }
})