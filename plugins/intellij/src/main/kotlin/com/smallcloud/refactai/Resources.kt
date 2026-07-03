package com.smallcloud.refactai

import com.intellij.openapi.application.ApplicationInfo
import com.intellij.openapi.extensions.PluginId
import com.intellij.openapi.util.IconLoader
import com.intellij.openapi.util.SystemInfo
import com.intellij.util.IconUtil
import java.io.File
import java.nio.file.Path
import javax.swing.Icon
import javax.swing.UIManager

private const val REFACT_PLUGIN_ID = "com.smallcloud.codify"
private const val FALLBACK_PLUGIN_VERSION = "8.2.3"

data class RefactPluginInfo(
    val pluginId: PluginId,
    val version: String,
    val pluginPath: Path?
)

fun getThisPlugin(): RefactPluginInfo? {
    return RefactPluginInfo(
        pluginId = PluginId.getId(REFACT_PLUGIN_ID),
        version = getVersion(),
        pluginPath = getHomePath()?.toPath()
    )
}

private fun getHomePath(): File? {
    val location = Resources::class.java.protectionDomain?.codeSource?.location ?: return null
    val file = runCatching { File(location.toURI()) }.getOrNull() ?: return null
    if (!file.isFile) return file

    val jarDir = file.parentFile ?: return null
    return if (jarDir.name == "lib") jarDir.parentFile ?: jarDir else jarDir
}

private fun getVersion(): String {
    return Resources::class.java.`package`?.implementationVersion?.takeIf { it.isNotBlank() }
        ?: FALLBACK_PLUGIN_VERSION
}

private fun getPluginId(): PluginId = PluginId.getId(REFACT_PLUGIN_ID)

private fun getArch(): String {
    val arch = SystemInfo.OS_ARCH
    return when (arch) {
        "amd64" -> "x86_64"
        "aarch64" -> "aarch64"
        else -> arch
    }
}

private fun getBinPrefix(): String {
    var suffix = ""
    if (SystemInfo.isMac) {
        suffix = "apple-darwin"
    } else if (SystemInfo.isWindows) {
        suffix = "pc-windows-msvc"
    } else if (SystemInfo.isLinux) {
        suffix = "unknown-linux-gnu"
    }

    return "dist-${getArch()}-${suffix}"
}

object Resources {
    val binPrefix: String = getBinPrefix()

    const val defaultCodeCompletionUrlSuffix: String = "v1/code-completion"
    val version: String = getVersion()
    const val client: String = "jetbrains"
    const val titleStr: String = "Refact.ai"
    val pluginId: PluginId = getPluginId()
    val jbBuildVersion: String = ApplicationInfo.getInstance().build.toString()
    const val refactAIRootSettingsID = "refactai_root"
    const val refactAIAdvancedSettingsID = "refactai_advanced_settings"

    object Icons {
        private fun brushForTheme(icon: Icon): Icon {
            return if (UIManager.getLookAndFeel().name.contains("Darcula")) {
                IconUtil.brighter(icon, 3)
            } else {
                IconUtil.darker(icon, 3)
            }
        }

        private fun makeIcon(path: String): Icon {
            return brushForTheme(IconLoader.getIcon(path, Resources::class.java))
        }

        val LOGO_RED_12x12: Icon = IconLoader.getIcon("/icons/refactai_logo_red_12x12.svg", Resources::class.java)
        val LOGO_RED_13x13: Icon = IconLoader.getIcon("/icons/refactai_logo_red_13x13.svg", Resources::class.java)
        val LOGO_12x12: Icon = makeIcon("/icons/refactai_logo_12x12.svg")
        val LOGO_RED_16x16: Icon = IconLoader.getIcon("/icons/refactai_logo_red_16x16.svg", Resources::class.java)

        val HAND_12x12: Icon = makeIcon("/icons/hand_12x12.svg")
    }
}
