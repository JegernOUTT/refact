package com.smallcloud.refactai.settings

import com.intellij.openapi.options.Configurable
import javax.swing.JComponent
import com.smallcloud.refactai.io.InferenceGlobalContext.Companion.instance as InferenceGlobalContext

/**
 * Provides controller functionality for application settings.
 */
class AppSettingsConfigurable : Configurable {
    private var mySettingsComponent: AppSettingsComponent? = null

    // A default constructor with no arguments is required because this implementation
    // is registered as an applicationConfigurable EP
    override fun getDisplayName(): String {
        return "Settings"
    }

    override fun getPreferredFocusedComponent(): JComponent {
        return mySettingsComponent!!.preferredFocusedComponent
    }

    override fun createComponent(): JComponent {
        mySettingsComponent = AppSettingsComponent()
        return mySettingsComponent!!.splitter
    }

    override fun isModified(): Boolean {
        var modified = false
        modified = modified || mySettingsComponent!!.useDeveloperMode != InferenceGlobalContext.developerModeEnabled

        modified = modified || mySettingsComponent!!.xDebugLSPPort != InferenceGlobalContext.xDebugLSPPort
        modified = modified || mySettingsComponent!!.refactBinaryPath != InferenceGlobalContext.refactBinaryPath

        modified = modified || mySettingsComponent!!.stagingVersion != InferenceGlobalContext.stagingVersion

        modified = modified || mySettingsComponent!!.astIsEnabled != InferenceGlobalContext.astIsEnabled
        modified = modified || mySettingsComponent!!.astFileLimit != InferenceGlobalContext.astFileLimit
        modified = modified || mySettingsComponent!!.codegraphIsEnabled != InferenceGlobalContext.codegraphIsEnabled
        modified = modified || mySettingsComponent!!.codegraphFileLimit != InferenceGlobalContext.codegraphFileLimit

        modified = modified || mySettingsComponent!!.insecureSSL != InferenceGlobalContext.insecureSSL
        modified = modified || mySettingsComponent!!.experimentalLspFlagEnabled != InferenceGlobalContext.experimentalLspFlagEnabled
        return modified
    }

    override fun apply() {
        InferenceGlobalContext.developerModeEnabled = mySettingsComponent!!.useDeveloperMode
        InferenceGlobalContext.stagingVersion = mySettingsComponent!!.stagingVersion
        InferenceGlobalContext.xDebugLSPPort = mySettingsComponent!!.xDebugLSPPort
        InferenceGlobalContext.refactBinaryPath = mySettingsComponent!!.refactBinaryPath
        InferenceGlobalContext.astIsEnabled = mySettingsComponent!!.astIsEnabled
        InferenceGlobalContext.astFileLimit = mySettingsComponent!!.astFileLimit
        InferenceGlobalContext.codegraphIsEnabled = mySettingsComponent!!.codegraphIsEnabled
        InferenceGlobalContext.codegraphFileLimit = mySettingsComponent!!.codegraphFileLimit
        InferenceGlobalContext.insecureSSL = mySettingsComponent!!.insecureSSL
        InferenceGlobalContext.experimentalLspFlagEnabled = mySettingsComponent!!.experimentalLspFlagEnabled
    }

    override fun reset() {
        mySettingsComponent!!.useDeveloperMode = InferenceGlobalContext.developerModeEnabled
        mySettingsComponent!!.stagingVersion = InferenceGlobalContext.stagingVersion
        mySettingsComponent!!.xDebugLSPPort = InferenceGlobalContext.xDebugLSPPort
        mySettingsComponent!!.refactBinaryPath = InferenceGlobalContext.refactBinaryPath
        mySettingsComponent!!.astIsEnabled = InferenceGlobalContext.astIsEnabled
        mySettingsComponent!!.astFileLimit = InferenceGlobalContext.astFileLimit
        mySettingsComponent!!.codegraphIsEnabled = InferenceGlobalContext.codegraphIsEnabled
        mySettingsComponent!!.codegraphFileLimit = InferenceGlobalContext.codegraphFileLimit
        mySettingsComponent!!.insecureSSL = InferenceGlobalContext.insecureSSL
        mySettingsComponent!!.experimentalLspFlagEnabled = InferenceGlobalContext.experimentalLspFlagEnabled
    }

    override fun disposeUIResources() {
        mySettingsComponent = null
    }


}
