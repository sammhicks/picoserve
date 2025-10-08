const counterOutput = document.getElementById("counterOutput");
const disconnectionMessage = document.getElementById("disconnectionMessage");
const shutdownButton = document.getElementById("shutdownButton");
const shutdownMessage = document.getElementById("shutdownMessage");

const events = new EventSource("counter");

events.addEventListener("tick", (ev) => {
    counterOutput.innerText = ev.data;
});

events.addEventListener("error", (ev) => {
    disconnectionMessage.innerText = "Connection Closed";
    events.close();
});


shutdownButton.addEventListener("click", async () => {
    const message = await (await fetch("shutdown", {
        method: "POST",
    })).text();

    shutdownMessage.innerText = message;
});