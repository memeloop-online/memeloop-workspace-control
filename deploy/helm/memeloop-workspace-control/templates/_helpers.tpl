{{- define "mwc.name" -}}
{{- printf "mwc-%s" (required "installationId is required" .Values.installationId) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "mwc.labels" -}}
app.kubernetes.io/name: memeloop-workspace-control
app.kubernetes.io/instance: {{ include "mwc.name" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
workspace.memeloop.dev/owner-installation: {{ required "installationId is required" .Values.installationId | quote }}
{{- end -}}

{{- define "mwc.selectorLabels" -}}
app.kubernetes.io/name: memeloop-workspace-control
app.kubernetes.io/instance: {{ include "mwc.name" . }}
{{- end -}}

{{- define "mwc.controlPlaneImage" -}}
{{- if .Values.image.digest -}}
{{- printf "%s@%s" .Values.image.repository .Values.image.digest -}}
{{- else -}}
{{- printf "%s:%s" .Values.image.repository .Values.image.tag -}}
{{- end -}}
{{- end -}}

{{- define "mwc.jumpHostImage" -}}
{{- if .Values.jumpHost.image.digest -}}
{{- printf "%s@%s" .Values.jumpHost.image.repository .Values.jumpHost.image.digest -}}
{{- else -}}
{{- printf "%s:%s" .Values.jumpHost.image.repository .Values.jumpHost.image.tag -}}
{{- end -}}
{{- end -}}

{{- define "mwc.validate" -}}
{{- if ne .Release.Namespace "memeloop-workspace-control" -}}
{{- fail "the release namespace must be memeloop-workspace-control" -}}
{{- end -}}
{{- if not (regexMatch "^[a-z0-9]([-a-z0-9]{0,18}[a-z0-9])?$" .Values.installationId) -}}
{{- fail "installationId must be a lower-case DNS label of at most 20 characters" -}}
{{- end -}}
{{- if not (has .Values.mode (list "sqlite" "postgresql")) -}}
{{- fail "mode must be sqlite or postgresql" -}}
{{- end -}}
{{- if and .Values.image.digest (not (regexMatch "^sha256:[0-9a-f]{64}$" .Values.image.digest)) -}}
{{- fail "image.digest must be an OCI sha256 digest" -}}
{{- end -}}
{{- if and .Values.jumpHost.image.digest (not (regexMatch "^sha256:[0-9a-f]{64}$" .Values.jumpHost.image.digest)) -}}
{{- fail "jumpHost.image.digest must be an OCI sha256 digest" -}}
{{- end -}}
{{- if and (eq .Values.mode "postgresql") (not .Values.database.postgresSecretName) -}}
{{- fail "database.postgresSecretName is required in postgresql mode" -}}
{{- end -}}
{{- if and .Values.sqlite.existingClaim (ne .Values.mode "sqlite") -}}
{{- fail "sqlite.existingClaim may only be set in sqlite mode" -}}
{{- end -}}
{{- if and .Values.sqlite.existingClaim .Values.sqlite.storageClassName -}}
{{- fail "sqlite.existingClaim and sqlite.storageClassName are mutually exclusive" -}}
{{- end -}}
{{- if and .Values.sqlite.existingClaim (or (gt (len .Values.sqlite.existingClaim) 253) (not (regexMatch "^[a-z0-9]([-a-z0-9]*[a-z0-9])?(\\.[a-z0-9]([-a-z0-9]*[a-z0-9])?)*$" .Values.sqlite.existingClaim))) -}}
{{- fail "sqlite.existingClaim must be a lower-case DNS subdomain of at most 253 characters" -}}
{{- end -}}
{{- if and (eq .Values.mode "postgresql") .Values.autoscaling.enabled -}}
{{- $cpuRequest := dig "requests" "cpu" "" .Values.resources -}}
{{- $memoryRequest := dig "requests" "memory" "" .Values.resources -}}
{{- if or (not $cpuRequest) (not $memoryRequest) -}}
{{- fail "resources.requests.cpu and resources.requests.memory are required when autoscaling is enabled" -}}
{{- end -}}
{{- end -}}
{{- if not .Values.secrets.encryptionSecretName -}}
{{- fail "secrets.encryptionSecretName is required" -}}
{{- end -}}
{{- if not .Values.secrets.internalAuthSecretName -}}
{{- fail "secrets.internalAuthSecretName is required" -}}
{{- end -}}
{{- if not .Values.workspace.ttydImage -}}
{{- fail "workspace.ttydImage must be an explicitly pinned image" -}}
{{- end -}}
{{- $ttydMtls := .Values.workspace.ttydMtls.serverTlsSecretName -}}
{{- $ttydMtlsClientCa := .Values.workspace.ttydMtls.clientCaSecretName -}}
{{- $higressMtlsNamespace := .Values.higress.ttydMtls.clientSecretNamespace -}}
{{- $higressMtlsName := .Values.higress.ttydMtls.clientSecretName -}}
{{- if or (and $ttydMtls (or (not $ttydMtlsClientCa) (not $higressMtlsNamespace) (not $higressMtlsName))) (and (not $ttydMtls) (or $ttydMtlsClientCa $higressMtlsNamespace $higressMtlsName)) -}}
{{- fail "workspace.ttydMtls serverTlsSecretName and clientCaSecretName, and higress.ttydMtls client Secret fields must be set together" -}}
{{- end -}}
{{- if and $ttydMtls (ne $higressMtlsNamespace .Values.higress.namespace) -}}
{{- fail "higress.ttydMtls.clientSecretNamespace must equal higress.namespace for exact EnvoyFilter ownership" -}}
{{- end -}}
{{- if or (not .Values.workspace.egress.dnsNamespace) (not (regexMatch "^[a-z0-9]([-a-z0-9]{0,61}[a-z0-9])?$" .Values.workspace.egress.dnsNamespace)) -}}
{{- fail "workspace.egress.dnsNamespace must be a lower-case DNS label" -}}
{{- end -}}
{{- if empty .Values.workspace.egress.dnsPodLabels -}}
{{- fail "workspace.egress.dnsPodLabels must select the DNS Pods" -}}
{{- end -}}
{{- range $cidr := .Values.workspace.egress.additionalBlockedCidrs -}}
{{- if not (regexMatch "^(([0-9]{1,3}[.]){3}[0-9]{1,3}|[0-9A-Fa-f:]+)/[0-9]{1,3}$" $cidr) -}}
{{- fail "workspace.egress.additionalBlockedCidrs entries must be CIDRs" -}}
{{- end -}}
{{- end -}}
{{- if and .Values.public.webShellDomain (empty .Values.higress.podLabels) -}}
{{- fail "higress.podLabels must identify the gateway Pods when public Web Shell is configured" -}}
{{- end -}}
{{- if and .Values.public.webShellDomain (not .Values.higress.extAuthPluginUrl) -}}
{{- fail "higress.extAuthPluginUrl is required when public.webShellDomain is configured" -}}
{{- end -}}
{{- if and .Values.public.webShellDomain (not .Values.networkPolicy.enabled) -}}
{{- fail "networkPolicy.enabled must remain true when public.webShellDomain is configured" -}}
{{- end -}}
{{- if and .Values.public.webShellDomain (ne .Values.public.webShellOrigin (printf "https://%s" .Values.public.webShellDomain)) -}}
{{- fail "public.webShellOrigin must equal https://<public.webShellDomain>" -}}
{{- end -}}
{{- if and .Values.public.webShellOrigin (not .Values.public.webShellDomain) -}}
{{- fail "public.webShellDomain is required when public.webShellOrigin is configured" -}}
{{- end -}}
{{- if and .Values.public.portMappingDomain (not .Values.higress.extAuthPluginUrl) -}}
{{- fail "higress.extAuthPluginUrl is required when public.portMappingDomain is configured" -}}
{{- end -}}
{{- if and .Values.public.portMappingDomain (not .Values.networkPolicy.enabled) -}}
{{- fail "networkPolicy.enabled must remain true when public.portMappingDomain is configured" -}}
{{- end -}}
{{- if and .Values.jumpHost.enabled (not .Values.jumpHost.hostKeySecretName) -}}
{{- fail "jumpHost.hostKeySecretName is required when jumpHost is enabled" -}}
{{- end -}}
{{- if and .Values.plugins.existingConfigMap .Values.plugins.existingClaim -}}
{{- fail "plugins.existingConfigMap and plugins.existingClaim are mutually exclusive" -}}
{{- end -}}
{{- if and .Values.plugins.existingConfigMap (empty .Values.plugins.configMapItems) -}}
{{- fail "plugins.configMapItems must map files into first-level package directories" -}}
{{- end -}}
{{- if and (not .Values.plugins.existingConfigMap) (not (empty .Values.plugins.configMapItems)) -}}
{{- fail "plugins.existingConfigMap is required when plugins.configMapItems is set" -}}
{{- end -}}
{{- end -}}
