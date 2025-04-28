// console.log("hello world")

import iohook from "iohook";

iohook.registerShortcut(["ctrl+alt+shift+s"], () => {
  console.log("shortcut pressed");
});

iohook.start();
