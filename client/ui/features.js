// EchoRoom 功能开关（构建前配置）
// 修改后需重新打包生效：cargo tauri build（dev 模式刷新即生效）
// 关闭的功能：界面不显示、行为不生效
window.FEATURES = {
  sound: true,      // 进出音效（含设置页"音效"分类）
  theme: true,      // 6 套主题（含外观页主题区）
  background: true, // 自定义背景图（含外观页背景图区）
};
