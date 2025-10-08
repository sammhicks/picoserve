let input = document.getElementsByTagName("input")[0];
let output = document.getElementById("output");
let button = document.getElementsByTagName("button")[0];

input.addEventListener("input", function () {
    button.disabled = !input.value;
});

const currentPath = window.location.pathname;

let websocketUri = (window.location.protocol === "https:") ? "wss:" : "ws:";
websocketUri += "//" + window.location.host;
websocketUri += currentPath.slice(0, currentPath.lastIndexOf("/") + 1) + "ws";

let ws = new WebSocket(websocketUri, ["echo", "ignored_protocol"]);

ws.addEventListener("close", function (ev) {
    ws.close();
    output.innerText = `Websockets Closed: ${ev.reason} (${ev.code})`;
})

ws.addEventListener("error", function (ev) {
    ws.close();
    console.error(ev);
    output.innerText = "Websockets Error";
});

ws.addEventListener("message", function (ev) {
    let message = document.createElement("li");
    message.innerText = ev.data;
    output.appendChild(message);
});

button.addEventListener("click", function () {
    ws.send(input.value);

    input.value = "";
});