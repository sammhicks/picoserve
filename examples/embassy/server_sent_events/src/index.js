let input = document.getElementsByTagName("input")[0];
let output = document.getElementsByTagName("output")[0];
let button = document.getElementsByTagName("button")[0];

input.addEventListener("input", function () {
  button.disabled = !input.value;
});

button.addEventListener("click", function () {
  fetch("set_message", {
    method: "POST",
    headers: {
      "Content-Type": "text/plain",
    },
    body: input.value,
  });

  input.value = "";
});

let events = new EventSource("events");

events.addEventListener("error", function (ev) {
  events.close();
  output.innerText = "Events Closed";
});

events.addEventListener("message_changed", function (ev) {
  output.innerText = ev.data;
})
