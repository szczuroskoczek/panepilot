import {
	registerHotkey,
	Modifiers,
	openWebview,
	registerAltRelease,
} from "$native";

const wv = openWebview("test", 200, 400);

let visible = false;
registerHotkey(Modifiers.Alt, 0x41, () => {
	if (visible) return;
	wv.setVisible(true);
	visible = true;
	registerAltRelease(() => {
		console.log("alt released");
		wv.setVisible(false);
		visible = false;
	});
});
