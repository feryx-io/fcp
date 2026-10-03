from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="ProviderCheck")



@_attrs_define
class ProviderCheck:
    """ What a provider's check found.

        Attributes:
            ok (bool): OIDC: the discovery document and keys were fetched; SAML: the
                metadata names a single sign-on address and a signing certificate.
            authorization_endpoint (None | str | Unset): Where people are sent to sign in.
            idp_entity_id (None | str | Unset): SAML: the provider's entity id, its logout address, and how many
                certificates it signs with.
            idp_logout_url (None | str | Unset):
            signing_certificates (int | None | Unset):
            error (None | str | Unset):
     """

    ok: bool
    authorization_endpoint: None | str | Unset = UNSET
    idp_entity_id: None | str | Unset = UNSET
    idp_logout_url: None | str | Unset = UNSET
    signing_certificates: int | None | Unset = UNSET
    error: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        ok = self.ok

        authorization_endpoint: None | str | Unset
        if isinstance(self.authorization_endpoint, Unset):
            authorization_endpoint = UNSET
        else:
            authorization_endpoint = self.authorization_endpoint

        idp_entity_id: None | str | Unset
        if isinstance(self.idp_entity_id, Unset):
            idp_entity_id = UNSET
        else:
            idp_entity_id = self.idp_entity_id

        idp_logout_url: None | str | Unset
        if isinstance(self.idp_logout_url, Unset):
            idp_logout_url = UNSET
        else:
            idp_logout_url = self.idp_logout_url

        signing_certificates: int | None | Unset
        if isinstance(self.signing_certificates, Unset):
            signing_certificates = UNSET
        else:
            signing_certificates = self.signing_certificates

        error: None | str | Unset
        if isinstance(self.error, Unset):
            error = UNSET
        else:
            error = self.error


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "ok": ok,
        })
        if authorization_endpoint is not UNSET:
            field_dict["authorization_endpoint"] = authorization_endpoint
        if idp_entity_id is not UNSET:
            field_dict["idp_entity_id"] = idp_entity_id
        if idp_logout_url is not UNSET:
            field_dict["idp_logout_url"] = idp_logout_url
        if signing_certificates is not UNSET:
            field_dict["signing_certificates"] = signing_certificates
        if error is not UNSET:
            field_dict["error"] = error

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        ok = d.pop("ok")

        def _parse_authorization_endpoint(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        authorization_endpoint = _parse_authorization_endpoint(d.pop("authorization_endpoint", UNSET))


        def _parse_idp_entity_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        idp_entity_id = _parse_idp_entity_id(d.pop("idp_entity_id", UNSET))


        def _parse_idp_logout_url(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        idp_logout_url = _parse_idp_logout_url(d.pop("idp_logout_url", UNSET))


        def _parse_signing_certificates(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        signing_certificates = _parse_signing_certificates(d.pop("signing_certificates", UNSET))


        def _parse_error(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        error = _parse_error(d.pop("error", UNSET))


        provider_check = cls(
            ok=ok,
            authorization_endpoint=authorization_endpoint,
            idp_entity_id=idp_entity_id,
            idp_logout_url=idp_logout_url,
            signing_certificates=signing_certificates,
            error=error,
        )


        provider_check.additional_properties = d
        return provider_check

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
