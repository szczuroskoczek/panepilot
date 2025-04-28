import { registerHotkey,  Modifiers } from "$native";

registerHotkey(Modifiers.Alt, 0x41, () => {
	console.log("Hello from hotkey");
});
