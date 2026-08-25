document.addEventListener("DOMContentLoaded", () => {
    const box = document.getElementById("counter-box");
    if (!box) return;

    const button = document.getElementById("counter-button");
    const output = document.getElementById("counter-output");

    button.addEventListener("click", () => {
        fetch("/session-info")
            .then((response) => response.json())
            .then((data) => {
                output.textContent =
                    "session_id=" + data.session_id +
                    "\nvisit_count=" + data.visit_count;
            })
            .catch((err) => {
                output.textContent = "request failed: " + err;
            });
    });
});
