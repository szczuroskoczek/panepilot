import { registerHotkey, Modifiers, openWebview } from "$native";

const wv = openWebview("test", 200, 400);

let visible = false;
registerHotkey(Modifiers.Alt, 0x41, () => {
	wv.setVisible(!visible);
	visible = !visible;
});
