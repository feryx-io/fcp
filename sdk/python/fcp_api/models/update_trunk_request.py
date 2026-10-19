from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="UpdateTrunkRequest")



@_attrs_define
class UpdateTrunkRequest:
    """ Update trunk request.

        Attributes:
            name (None | str | Unset):
            enabled (bool | None | Unset):
            direction (None | str | Unset):
            priority (int | None | Unset):
            weight (int | None | Unset):
            hosts (list[str] | None | Unset):
            inbound_sources (list[str] | None | Unset):
            site_id (None | str | Unset):
            transport_protocol (None | str | Unset):
            transport_port (int | None | Unset):
            verify_tls (bool | None | Unset):
            auth_username (None | str | Unset):
            auth_password (None | str | Unset):
            auth_realm (None | str | Unset):
            register (bool | None | Unset):
            register_expiry (int | None | Unset):
            max_concurrent_calls (int | None | Unset):
            max_failures (int | None | Unset):
            recovery_interval_secs (int | None | Unset):
            health_check_enabled (bool | None | Unset):
            emergency_capable (bool | None | Unset):
            health_check_interval_secs (int | None | Unset):
            codec_prefs (list[str] | None | Unset):
            caller_id_override (None | str | Unset):
            from_domain (None | str | Unset):
            strip_digits (int | None | Unset):
            prepend_digits (None | str | Unset):
            send_e164 (bool | None | Unset): Send numbers in E.164, before the digit plan.
            cps_limit (int | None | Unset):
            description (None | str | Unset):
            srtp_mode (None | str | Unset):
            dtmf_mode (None | str | Unset):
            session_timer_secs (int | None | Unset):
            t38_enabled (bool | None | Unset):
            outbound_proxy (None | str | Unset):
            trunk_group (None | str | Unset):
            header_rules (None | str | Unset):
     """

    name: None | str | Unset = UNSET
    enabled: bool | None | Unset = UNSET
    direction: None | str | Unset = UNSET
    priority: int | None | Unset = UNSET
    weight: int | None | Unset = UNSET
    hosts: list[str] | None | Unset = UNSET
    inbound_sources: list[str] | None | Unset = UNSET
    site_id: None | str | Unset = UNSET
    transport_protocol: None | str | Unset = UNSET
    transport_port: int | None | Unset = UNSET
    verify_tls: bool | None | Unset = UNSET
    auth_username: None | str | Unset = UNSET
    auth_password: None | str | Unset = UNSET
    auth_realm: None | str | Unset = UNSET
    register: bool | None | Unset = UNSET
    register_expiry: int | None | Unset = UNSET
    max_concurrent_calls: int | None | Unset = UNSET
    max_failures: int | None | Unset = UNSET
    recovery_interval_secs: int | None | Unset = UNSET
    health_check_enabled: bool | None | Unset = UNSET
    emergency_capable: bool | None | Unset = UNSET
    health_check_interval_secs: int | None | Unset = UNSET
    codec_prefs: list[str] | None | Unset = UNSET
    caller_id_override: None | str | Unset = UNSET
    from_domain: None | str | Unset = UNSET
    strip_digits: int | None | Unset = UNSET
    prepend_digits: None | str | Unset = UNSET
    send_e164: bool | None | Unset = UNSET
    cps_limit: int | None | Unset = UNSET
    description: None | str | Unset = UNSET
    srtp_mode: None | str | Unset = UNSET
    dtmf_mode: None | str | Unset = UNSET
    session_timer_secs: int | None | Unset = UNSET
    t38_enabled: bool | None | Unset = UNSET
    outbound_proxy: None | str | Unset = UNSET
    trunk_group: None | str | Unset = UNSET
    header_rules: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
            name = self.name

        enabled: bool | None | Unset
        if isinstance(self.enabled, Unset):
            enabled = UNSET
        else:
            enabled = self.enabled

        direction: None | str | Unset
        if isinstance(self.direction, Unset):
            direction = UNSET
        else:
            direction = self.direction

        priority: int | None | Unset
        if isinstance(self.priority, Unset):
            priority = UNSET
        else:
            priority = self.priority

        weight: int | None | Unset
        if isinstance(self.weight, Unset):
            weight = UNSET
        else:
            weight = self.weight

        hosts: list[str] | None | Unset
        if isinstance(self.hosts, Unset):
            hosts = UNSET
        elif isinstance(self.hosts, list):
            hosts = self.hosts


        else:
            hosts = self.hosts

        inbound_sources: list[str] | None | Unset
        if isinstance(self.inbound_sources, Unset):
            inbound_sources = UNSET
        elif isinstance(self.inbound_sources, list):
            inbound_sources = self.inbound_sources


        else:
            inbound_sources = self.inbound_sources

        site_id: None | str | Unset
        if isinstance(self.site_id, Unset):
            site_id = UNSET
        else:
            site_id = self.site_id

        transport_protocol: None | str | Unset
        if isinstance(self.transport_protocol, Unset):
            transport_protocol = UNSET
        else:
            transport_protocol = self.transport_protocol

        transport_port: int | None | Unset
        if isinstance(self.transport_port, Unset):
            transport_port = UNSET
        else:
            transport_port = self.transport_port

        verify_tls: bool | None | Unset
        if isinstance(self.verify_tls, Unset):
            verify_tls = UNSET
        else:
            verify_tls = self.verify_tls

        auth_username: None | str | Unset
        if isinstance(self.auth_username, Unset):
            auth_username = UNSET
        else:
            auth_username = self.auth_username

        auth_password: None | str | Unset
        if isinstance(self.auth_password, Unset):
            auth_password = UNSET
        else:
            auth_password = self.auth_password

        auth_realm: None | str | Unset
        if isinstance(self.auth_realm, Unset):
            auth_realm = UNSET
        else:
            auth_realm = self.auth_realm

        register: bool | None | Unset
        if isinstance(self.register, Unset):
            register = UNSET
        else:
            register = self.register

        register_expiry: int | None | Unset
        if isinstance(self.register_expiry, Unset):
            register_expiry = UNSET
        else:
            register_expiry = self.register_expiry

        max_concurrent_calls: int | None | Unset
        if isinstance(self.max_concurrent_calls, Unset):
            max_concurrent_calls = UNSET
        else:
            max_concurrent_calls = self.max_concurrent_calls

        max_failures: int | None | Unset
        if isinstance(self.max_failures, Unset):
            max_failures = UNSET
        else:
            max_failures = self.max_failures

        recovery_interval_secs: int | None | Unset
        if isinstance(self.recovery_interval_secs, Unset):
            recovery_interval_secs = UNSET
        else:
            recovery_interval_secs = self.recovery_interval_secs

        health_check_enabled: bool | None | Unset
        if isinstance(self.health_check_enabled, Unset):
            health_check_enabled = UNSET
        else:
            health_check_enabled = self.health_check_enabled

        emergency_capable: bool | None | Unset
        if isinstance(self.emergency_capable, Unset):
            emergency_capable = UNSET
        else:
            emergency_capable = self.emergency_capable

        health_check_interval_secs: int | None | Unset
        if isinstance(self.health_check_interval_secs, Unset):
            health_check_interval_secs = UNSET
        else:
            health_check_interval_secs = self.health_check_interval_secs

        codec_prefs: list[str] | None | Unset
        if isinstance(self.codec_prefs, Unset):
            codec_prefs = UNSET
        elif isinstance(self.codec_prefs, list):
            codec_prefs = self.codec_prefs


        else:
            codec_prefs = self.codec_prefs

        caller_id_override: None | str | Unset
        if isinstance(self.caller_id_override, Unset):
            caller_id_override = UNSET
        else:
            caller_id_override = self.caller_id_override

        from_domain: None | str | Unset
        if isinstance(self.from_domain, Unset):
            from_domain = UNSET
        else:
            from_domain = self.from_domain

        strip_digits: int | None | Unset
        if isinstance(self.strip_digits, Unset):
            strip_digits = UNSET
        else:
            strip_digits = self.strip_digits

        prepend_digits: None | str | Unset
        if isinstance(self.prepend_digits, Unset):
            prepend_digits = UNSET
        else:
            prepend_digits = self.prepend_digits

        send_e164: bool | None | Unset
        if isinstance(self.send_e164, Unset):
            send_e164 = UNSET
        else:
            send_e164 = self.send_e164

        cps_limit: int | None | Unset
        if isinstance(self.cps_limit, Unset):
            cps_limit = UNSET
        else:
            cps_limit = self.cps_limit

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description

        srtp_mode: None | str | Unset
        if isinstance(self.srtp_mode, Unset):
            srtp_mode = UNSET
        else:
            srtp_mode = self.srtp_mode

        dtmf_mode: None | str | Unset
        if isinstance(self.dtmf_mode, Unset):
            dtmf_mode = UNSET
        else:
            dtmf_mode = self.dtmf_mode

        session_timer_secs: int | None | Unset
        if isinstance(self.session_timer_secs, Unset):
            session_timer_secs = UNSET
        else:
            session_timer_secs = self.session_timer_secs

        t38_enabled: bool | None | Unset
        if isinstance(self.t38_enabled, Unset):
            t38_enabled = UNSET
        else:
            t38_enabled = self.t38_enabled

        outbound_proxy: None | str | Unset
        if isinstance(self.outbound_proxy, Unset):
            outbound_proxy = UNSET
        else:
            outbound_proxy = self.outbound_proxy

        trunk_group: None | str | Unset
        if isinstance(self.trunk_group, Unset):
            trunk_group = UNSET
        else:
            trunk_group = self.trunk_group

        header_rules: None | str | Unset
        if isinstance(self.header_rules, Unset):
            header_rules = UNSET
        else:
            header_rules = self.header_rules


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if name is not UNSET:
            field_dict["name"] = name
        if enabled is not UNSET:
            field_dict["enabled"] = enabled
        if direction is not UNSET:
            field_dict["direction"] = direction
        if priority is not UNSET:
            field_dict["priority"] = priority
        if weight is not UNSET:
            field_dict["weight"] = weight
        if hosts is not UNSET:
            field_dict["hosts"] = hosts
        if inbound_sources is not UNSET:
            field_dict["inbound_sources"] = inbound_sources
        if site_id is not UNSET:
            field_dict["site_id"] = site_id
        if transport_protocol is not UNSET:
            field_dict["transport_protocol"] = transport_protocol
        if transport_port is not UNSET:
            field_dict["transport_port"] = transport_port
        if verify_tls is not UNSET:
            field_dict["verify_tls"] = verify_tls
        if auth_username is not UNSET:
            field_dict["auth_username"] = auth_username
        if auth_password is not UNSET:
            field_dict["auth_password"] = auth_password
        if auth_realm is not UNSET:
            field_dict["auth_realm"] = auth_realm
        if register is not UNSET:
            field_dict["register"] = register
        if register_expiry is not UNSET:
            field_dict["register_expiry"] = register_expiry
        if max_concurrent_calls is not UNSET:
            field_dict["max_concurrent_calls"] = max_concurrent_calls
        if max_failures is not UNSET:
            field_dict["max_failures"] = max_failures
        if recovery_interval_secs is not UNSET:
            field_dict["recovery_interval_secs"] = recovery_interval_secs
        if health_check_enabled is not UNSET:
            field_dict["health_check_enabled"] = health_check_enabled
        if emergency_capable is not UNSET:
            field_dict["emergency_capable"] = emergency_capable
        if health_check_interval_secs is not UNSET:
            field_dict["health_check_interval_secs"] = health_check_interval_secs
        if codec_prefs is not UNSET:
            field_dict["codec_prefs"] = codec_prefs
        if caller_id_override is not UNSET:
            field_dict["caller_id_override"] = caller_id_override
        if from_domain is not UNSET:
            field_dict["from_domain"] = from_domain
        if strip_digits is not UNSET:
            field_dict["strip_digits"] = strip_digits
        if prepend_digits is not UNSET:
            field_dict["prepend_digits"] = prepend_digits
        if send_e164 is not UNSET:
            field_dict["send_e164"] = send_e164
        if cps_limit is not UNSET:
            field_dict["cps_limit"] = cps_limit
        if description is not UNSET:
            field_dict["description"] = description
        if srtp_mode is not UNSET:
            field_dict["srtp_mode"] = srtp_mode
        if dtmf_mode is not UNSET:
            field_dict["dtmf_mode"] = dtmf_mode
        if session_timer_secs is not UNSET:
            field_dict["session_timer_secs"] = session_timer_secs
        if t38_enabled is not UNSET:
            field_dict["t38_enabled"] = t38_enabled
        if outbound_proxy is not UNSET:
            field_dict["outbound_proxy"] = outbound_proxy
        if trunk_group is not UNSET:
            field_dict["trunk_group"] = trunk_group
        if header_rules is not UNSET:
            field_dict["header_rules"] = header_rules

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


        def _parse_enabled(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        enabled = _parse_enabled(d.pop("enabled", UNSET))


        def _parse_direction(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        direction = _parse_direction(d.pop("direction", UNSET))


        def _parse_priority(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        priority = _parse_priority(d.pop("priority", UNSET))


        def _parse_weight(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        weight = _parse_weight(d.pop("weight", UNSET))


        def _parse_hosts(data: object) -> list[str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                hosts_type_0 = cast(list[str], data)

                return hosts_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[str] | None | Unset, data)

        hosts = _parse_hosts(d.pop("hosts", UNSET))


        def _parse_inbound_sources(data: object) -> list[str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                inbound_sources_type_0 = cast(list[str], data)

                return inbound_sources_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[str] | None | Unset, data)

        inbound_sources = _parse_inbound_sources(d.pop("inbound_sources", UNSET))


        def _parse_site_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        site_id = _parse_site_id(d.pop("site_id", UNSET))


        def _parse_transport_protocol(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        transport_protocol = _parse_transport_protocol(d.pop("transport_protocol", UNSET))


        def _parse_transport_port(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        transport_port = _parse_transport_port(d.pop("transport_port", UNSET))


        def _parse_verify_tls(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        verify_tls = _parse_verify_tls(d.pop("verify_tls", UNSET))


        def _parse_auth_username(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        auth_username = _parse_auth_username(d.pop("auth_username", UNSET))


        def _parse_auth_password(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        auth_password = _parse_auth_password(d.pop("auth_password", UNSET))


        def _parse_auth_realm(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        auth_realm = _parse_auth_realm(d.pop("auth_realm", UNSET))


        def _parse_register(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        register = _parse_register(d.pop("register", UNSET))


        def _parse_register_expiry(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        register_expiry = _parse_register_expiry(d.pop("register_expiry", UNSET))


        def _parse_max_concurrent_calls(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        max_concurrent_calls = _parse_max_concurrent_calls(d.pop("max_concurrent_calls", UNSET))


        def _parse_max_failures(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        max_failures = _parse_max_failures(d.pop("max_failures", UNSET))


        def _parse_recovery_interval_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        recovery_interval_secs = _parse_recovery_interval_secs(d.pop("recovery_interval_secs", UNSET))


        def _parse_health_check_enabled(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        health_check_enabled = _parse_health_check_enabled(d.pop("health_check_enabled", UNSET))


        def _parse_emergency_capable(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        emergency_capable = _parse_emergency_capable(d.pop("emergency_capable", UNSET))


        def _parse_health_check_interval_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        health_check_interval_secs = _parse_health_check_interval_secs(d.pop("health_check_interval_secs", UNSET))


        def _parse_codec_prefs(data: object) -> list[str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                codec_prefs_type_0 = cast(list[str], data)

                return codec_prefs_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[str] | None | Unset, data)

        codec_prefs = _parse_codec_prefs(d.pop("codec_prefs", UNSET))


        def _parse_caller_id_override(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        caller_id_override = _parse_caller_id_override(d.pop("caller_id_override", UNSET))


        def _parse_from_domain(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        from_domain = _parse_from_domain(d.pop("from_domain", UNSET))


        def _parse_strip_digits(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        strip_digits = _parse_strip_digits(d.pop("strip_digits", UNSET))


        def _parse_prepend_digits(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        prepend_digits = _parse_prepend_digits(d.pop("prepend_digits", UNSET))


        def _parse_send_e164(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        send_e164 = _parse_send_e164(d.pop("send_e164", UNSET))


        def _parse_cps_limit(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        cps_limit = _parse_cps_limit(d.pop("cps_limit", UNSET))


        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        def _parse_srtp_mode(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        srtp_mode = _parse_srtp_mode(d.pop("srtp_mode", UNSET))


        def _parse_dtmf_mode(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        dtmf_mode = _parse_dtmf_mode(d.pop("dtmf_mode", UNSET))


        def _parse_session_timer_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        session_timer_secs = _parse_session_timer_secs(d.pop("session_timer_secs", UNSET))


        def _parse_t38_enabled(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        t38_enabled = _parse_t38_enabled(d.pop("t38_enabled", UNSET))


        def _parse_outbound_proxy(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        outbound_proxy = _parse_outbound_proxy(d.pop("outbound_proxy", UNSET))


        def _parse_trunk_group(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        trunk_group = _parse_trunk_group(d.pop("trunk_group", UNSET))


        def _parse_header_rules(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        header_rules = _parse_header_rules(d.pop("header_rules", UNSET))


        update_trunk_request = cls(
            name=name,
            enabled=enabled,
            direction=direction,
            priority=priority,
            weight=weight,
            hosts=hosts,
            inbound_sources=inbound_sources,
            site_id=site_id,
            transport_protocol=transport_protocol,
            transport_port=transport_port,
            verify_tls=verify_tls,
            auth_username=auth_username,
            auth_password=auth_password,
            auth_realm=auth_realm,
            register=register,
            register_expiry=register_expiry,
            max_concurrent_calls=max_concurrent_calls,
            max_failures=max_failures,
            recovery_interval_secs=recovery_interval_secs,
            health_check_enabled=health_check_enabled,
            emergency_capable=emergency_capable,
            health_check_interval_secs=health_check_interval_secs,
            codec_prefs=codec_prefs,
            caller_id_override=caller_id_override,
            from_domain=from_domain,
            strip_digits=strip_digits,
            prepend_digits=prepend_digits,
            send_e164=send_e164,
            cps_limit=cps_limit,
            description=description,
            srtp_mode=srtp_mode,
            dtmf_mode=dtmf_mode,
            session_timer_secs=session_timer_secs,
            t38_enabled=t38_enabled,
            outbound_proxy=outbound_proxy,
            trunk_group=trunk_group,
            header_rules=header_rules,
        )


        update_trunk_request.additional_properties = d
        return update_trunk_request

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
