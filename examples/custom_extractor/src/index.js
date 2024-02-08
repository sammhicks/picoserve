async function send() {
    let response = await fetch("/number", {
        method: "POST",
        headers: {
            "content-type": "text/plain"
        },
        body: document.getElementById("data").value
    });

    document.getElementById("response").innerText = await response.text();
}