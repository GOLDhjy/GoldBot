# Global Shader（VS/PS）参考

用于插件内普通 Global Shader（Vertex + Pixel）的最小骨架与检查项。

## 1. 模块初始化（Plugin Shader 路径映射）

在模块 `StartupModule()` 中注册插件 Shader 目录。

```cpp
// Copyright Epic Games, Inc. All Rights Reserved.

#include "Interfaces/IPluginManager.h"
#include "Misc/Paths.h"
#include "ShaderCore.h"

void FMyPluginModule::StartupModule()
{
	const FString PluginShaderDir = FPaths::Combine(
		IPluginManager::Get().FindPlugin(TEXT("MyPlugin"))->GetBaseDir(),
		TEXT("Shaders"));
	AddShaderSourceDirectoryMapping(TEXT("/Plugin/MyPlugin"), PluginShaderDir);
}
```

`IMPLEMENT_GLOBAL_SHADER(...)` 里的虚拟路径要与这里的映射一致。

## 2. Shader 类模板（VS / PS）

```cpp
class FMyExampleVS : public FGlobalShader
{
	DECLARE_GLOBAL_SHADER(FMyExampleVS);
	SHADER_USE_PARAMETER_STRUCT(FMyExampleVS, FGlobalShader);

public:
	static bool ShouldCompilePermutation(const FGlobalShaderPermutationParameters& Parameters)
	{
		return IsFeatureLevelSupported(Parameters.Platform, ERHIFeatureLevel::SM5);
	}

	BEGIN_SHADER_PARAMETER_STRUCT(FParameters, )
		SHADER_PARAMETER(FVector4f, TintColor)
	END_SHADER_PARAMETER_STRUCT()
};

class FMyExamplePS : public FGlobalShader
{
	DECLARE_GLOBAL_SHADER(FMyExamplePS);
	SHADER_USE_PARAMETER_STRUCT(FMyExamplePS, FGlobalShader);

public:
	static bool ShouldCompilePermutation(const FGlobalShaderPermutationParameters& Parameters)
	{
		return IsFeatureLevelSupported(Parameters.Platform, ERHIFeatureLevel::SM5);
	}

	BEGIN_SHADER_PARAMETER_STRUCT(FParameters, )
		SHADER_PARAMETER(FVector4f, TintColor)
		RENDER_TARGET_BINDING_SLOTS()
	END_SHADER_PARAMETER_STRUCT()
};

IMPLEMENT_GLOBAL_SHADER(FMyExampleVS, "/Plugin/MyPlugin/Private/MyExample.usf", "MainVS", SF_Vertex);
IMPLEMENT_GLOBAL_SHADER(FMyExamplePS, "/Plugin/MyPlugin/Private/MyExample.usf", "MainPS", SF_Pixel);
```

Notes:

1. 写 Render Target 的 Pixel Pass 通常需要 `RENDER_TARGET_BINDING_SLOTS()`
2. 只有确实需要宏定义/编译环境时再加 `ModifyCompilationEnvironment(...)`
3. 插件里已有风格时优先复用本地风格

## 3. 最小 `.usf` 模板

File: `Shaders/Private/MyExample.usf`

```hlsl
struct FVSOutput
{
	float4 Position : SV_POSITION;
	float2 UV       : TEXCOORD0;
};

FVSOutput MainVS(uint VertexId : SV_VertexID)
{
	FVSOutput Out;

	float2 Pos[3] =
	{
		float2(-1.0, -1.0),
		float2(-1.0,  3.0),
		float2( 3.0, -1.0)
	};

	float2 UVs[3] =
	{
		float2(0.0, 1.0),
		float2(0.0, -1.0),
		float2(2.0, 1.0)
	};

	Out.Position = float4(Pos[VertexId], 0.0, 1.0);
	Out.UV = UVs[VertexId];
	return Out;
}

float4 MainPS(FVSOutput In) : SV_Target0
{
	return float4(In.UV, 0.0, 1.0);
}
```

## 4. 调用点说明

调用点 API 会随 UE 版本和渲染路径变化，优先抄本地同版本示例。

建议按下面清单接入：

1. 从 `FRDGBuilder` 分配参数结构体
2. 在 `FMyExamplePS::FParameters` 里绑定 Render Target
3. 使用仓库已有的全屏绘制辅助模式（如 `FPixelShaderUtils` 或本地封装）
4. 保证执行发生在渲染线程或 RDG Pass 中

## 5. 常见错误

1. `.usf` 里函数叫 `MainPS`，但 `IMPLEMENT_GLOBAL_SHADER(..., "MainPixel", ...)` 写了别的名字
2. 文件在 `Shaders/Private/`，虚拟路径却写成 `/Project/...` 而不是 `/Plugin/MyPlugin/...`
3. `SF_Vertex` / `SF_Pixel` 对应的 entry point 写错
