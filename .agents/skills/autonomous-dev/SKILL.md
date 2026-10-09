---
name: autonomous-dev
description: >-
  在 scroll-shot 项目开发过程中指导自主开发行为准则：具备独立思考、主动联网查询、编译/运行时自动纠错排查的能力，且在项目目录下的命令直接执行而无需反复咨询确认。
---

# Autonomous Development Workflow for scroll-shot

本项目（`scroll-shot`）开发时的核心行为规范与自动化准则：

## 1. 独立思考与自主决策 (Independent Thinking)
- 遇到架构设计、算法选型（如 1D 模板匹配、XShm 抓取、XTest 模拟滚动）等技术细节时，深入思考并给出最优实现方案。
- 主动分析边界情况（如窗口滚动到底、页面含 Sticky Header、高分辨率 HiDPI 缩放比例对齐、内存占用等）。

## 2. 联网查询 (Web Search & Research)
- 遇到底层协议细节（如 X11 MIT-SHM 零拷贝、XTest 扩展、x11rb 具体 API 用法、Linux 剪贴板协议规范）或特定 crate 文档不明确时，主动使用联网搜索工具进行查询与验证。

## 3. 自动纠错与闭环排查 (Auto Error Correction)
- 在执行构建（`cargo build`）、单元测试或运行时测试若遇到报错，不要中断询问用户。
- 自动分析编译器报错信息与堆栈跟踪，自主修改源码或调整配置，直至构建与测试通过。

## 4. 命令自主执行 (Autonomous Command Execution)
- 在本项目目录（`/home/ephm/Documents/Projects/scroll-shot`）下进行的操作（包括安装 Rust 工具链、初始化项目、添加依赖、构建、测试与运行等），**直接执行即可，无需频繁询问或征求权限**。
- 仅在遇到可能破坏系统环境、涉及外部危险操作或出现重大业务逻辑歧义时，再与用户确认。
