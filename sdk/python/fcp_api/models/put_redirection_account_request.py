from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="PutRedirectionAccountRequest")



@_attrs_define
class PutRedirectionAccountRequest:
    """ An account, made or changed.

        Attributes:
            key_id (str):
            enabled (bool | None | Unset): Unset: on.
            secret (None | str | Unset): Required to make the account; unset on a change keeps it.
            account_id (None | str | Unset):
            profile (None | str | Unset):
     """

    key_id: str
    enabled: bool | None | Unset = UNSET
    secret: None | str | Unset = UNSET
    account_id: None | str | Unset = UNSET
    profile: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        key_id = self.key_id

        enabled: bool | None | Unset
        if isinstance(self.enabled, Unset):
            enabled = UNSET
        else:
            enabled = self.enabled

        secret: None | str | Unset
        if isinstance(self.secret, Unset):
            secret = UNSET
        else:
            secret = self.secret

        account_id: None | str | Unset
        if isinstance(self.account_id, Unset):
            account_id = UNSET
        else:
            account_id = self.account_id

        profile: None | str | Unset
        if isinstance(self.profile, Unset):
            profile = UNSET
        else:
            profile = self.profile


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "key_id": key_id,
        })
        if enabled is not UNSET:
            field_dict["enabled"] = enabled
        if secret is not UNSET:
            field_dict["secret"] = secret
        if account_id is not UNSET:
            field_dict["account_id"] = account_id
        if profile is not UNSET:
            field_dict["profile"] = profile

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        key_id = d.pop("key_id")

        def _parse_enabled(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        enabled = _parse_enabled(d.pop("enabled", UNSET))


        def _parse_secret(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        secret = _parse_secret(d.pop("secret", UNSET))


        def _parse_account_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        account_id = _parse_account_id(d.pop("account_id", UNSET))


        def _parse_profile(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        profile = _parse_profile(d.pop("profile", UNSET))


        put_redirection_account_request = cls(
            key_id=key_id,
            enabled=enabled,
            secret=secret,
            account_id=account_id,
            profile=profile,
        )


        put_redirection_account_request.additional_properties = d
        return put_redirection_account_request

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
