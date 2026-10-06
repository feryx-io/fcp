from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="SoftphoneAccount")



@_attrs_define
class SoftphoneAccount:
    """ The SIP account the app registers with.

        Attributes:
            aor (str): The address of record: the user in the tenant's domain.
            domain (str): The tenant's SIP domain.
            registrar (str): The Request-URI of a REGISTER: `sip:<domain>`.
            transport (str): `udp`, `tcp` or `tls`.
            realm (str): The realm the password is hashed in.
            sip_username (str): The device's own SIP username; the password came with the sign-in.
            register_expires (int): Seconds a registration is asked for.
            display_name (None | str | Unset):
            outbound_proxy (None | str | Unset): Where every request goes when the domain is not the server's
                address: `sip:<server>:<port>;transport=<transport>`.
     """

    aor: str
    domain: str
    registrar: str
    transport: str
    realm: str
    sip_username: str
    register_expires: int
    display_name: None | str | Unset = UNSET
    outbound_proxy: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        aor = self.aor

        domain = self.domain

        registrar = self.registrar

        transport = self.transport

        realm = self.realm

        sip_username = self.sip_username

        register_expires = self.register_expires

        display_name: None | str | Unset
        if isinstance(self.display_name, Unset):
            display_name = UNSET
        else:
            display_name = self.display_name

        outbound_proxy: None | str | Unset
        if isinstance(self.outbound_proxy, Unset):
            outbound_proxy = UNSET
        else:
            outbound_proxy = self.outbound_proxy


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "aor": aor,
            "domain": domain,
            "registrar": registrar,
            "transport": transport,
            "realm": realm,
            "sip_username": sip_username,
            "register_expires": register_expires,
        })
        if display_name is not UNSET:
            field_dict["display_name"] = display_name
        if outbound_proxy is not UNSET:
            field_dict["outbound_proxy"] = outbound_proxy

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        aor = d.pop("aor")

        domain = d.pop("domain")

        registrar = d.pop("registrar")

        transport = d.pop("transport")

        realm = d.pop("realm")

        sip_username = d.pop("sip_username")

        register_expires = d.pop("register_expires")

        def _parse_display_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        display_name = _parse_display_name(d.pop("display_name", UNSET))


        def _parse_outbound_proxy(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        outbound_proxy = _parse_outbound_proxy(d.pop("outbound_proxy", UNSET))


        softphone_account = cls(
            aor=aor,
            domain=domain,
            registrar=registrar,
            transport=transport,
            realm=realm,
            sip_username=sip_username,
            register_expires=register_expires,
            display_name=display_name,
            outbound_proxy=outbound_proxy,
        )


        softphone_account.additional_properties = d
        return softphone_account

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
