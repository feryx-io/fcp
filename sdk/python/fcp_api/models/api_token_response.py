from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="ApiTokenResponse")



@_attrs_define
class ApiTokenResponse:
    """ Response model for an API token (never includes a hash).

        Attributes:
            id (str):
            name (str):
            prefix (str):
            scope (str): `admin` (the admin API under its permissions) or `personal` (its
                maker's own `/me` routes).
            created_by (str): The maker's username; empty for a token made with the static token or
                before makers were recorded.
            tenant_id (str):
            permissions (list[str]):
            legacy_full_access (bool): A token made before permissions were checked, carrying `admin` or
                `*`. Since v0.25.498 it reaches what a tenant administrator may (or
                its maker, when it has one), never system configuration: make a new
                one with the permissions it needs.
            created_at (str):
            expires_at (None | str | Unset):
            last_used_at (None | str | Unset):
            revoked_at (None | str | Unset):
            revoked_by (None | str | Unset):
            rotated_at (None | str | Unset):
            previous_secret_valid_until (None | str | Unset): Until when the secret a rotation replaced still opens the
                token.
     """

    id: str
    name: str
    prefix: str
    scope: str
    created_by: str
    tenant_id: str
    permissions: list[str]
    legacy_full_access: bool
    created_at: str
    expires_at: None | str | Unset = UNSET
    last_used_at: None | str | Unset = UNSET
    revoked_at: None | str | Unset = UNSET
    revoked_by: None | str | Unset = UNSET
    rotated_at: None | str | Unset = UNSET
    previous_secret_valid_until: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        name = self.name

        prefix = self.prefix

        scope = self.scope

        created_by = self.created_by

        tenant_id = self.tenant_id

        permissions = self.permissions



        legacy_full_access = self.legacy_full_access

        created_at = self.created_at

        expires_at: None | str | Unset
        if isinstance(self.expires_at, Unset):
            expires_at = UNSET
        else:
            expires_at = self.expires_at

        last_used_at: None | str | Unset
        if isinstance(self.last_used_at, Unset):
            last_used_at = UNSET
        else:
            last_used_at = self.last_used_at

        revoked_at: None | str | Unset
        if isinstance(self.revoked_at, Unset):
            revoked_at = UNSET
        else:
            revoked_at = self.revoked_at

        revoked_by: None | str | Unset
        if isinstance(self.revoked_by, Unset):
            revoked_by = UNSET
        else:
            revoked_by = self.revoked_by

        rotated_at: None | str | Unset
        if isinstance(self.rotated_at, Unset):
            rotated_at = UNSET
        else:
            rotated_at = self.rotated_at

        previous_secret_valid_until: None | str | Unset
        if isinstance(self.previous_secret_valid_until, Unset):
            previous_secret_valid_until = UNSET
        else:
            previous_secret_valid_until = self.previous_secret_valid_until


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "name": name,
            "prefix": prefix,
            "scope": scope,
            "created_by": created_by,
            "tenant_id": tenant_id,
            "permissions": permissions,
            "legacy_full_access": legacy_full_access,
            "created_at": created_at,
        })
        if expires_at is not UNSET:
            field_dict["expires_at"] = expires_at
        if last_used_at is not UNSET:
            field_dict["last_used_at"] = last_used_at
        if revoked_at is not UNSET:
            field_dict["revoked_at"] = revoked_at
        if revoked_by is not UNSET:
            field_dict["revoked_by"] = revoked_by
        if rotated_at is not UNSET:
            field_dict["rotated_at"] = rotated_at
        if previous_secret_valid_until is not UNSET:
            field_dict["previous_secret_valid_until"] = previous_secret_valid_until

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        name = d.pop("name")

        prefix = d.pop("prefix")

        scope = d.pop("scope")

        created_by = d.pop("created_by")

        tenant_id = d.pop("tenant_id")

        permissions = cast(list[str], d.pop("permissions"))


        legacy_full_access = d.pop("legacy_full_access")

        created_at = d.pop("created_at")

        def _parse_expires_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        expires_at = _parse_expires_at(d.pop("expires_at", UNSET))


        def _parse_last_used_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        last_used_at = _parse_last_used_at(d.pop("last_used_at", UNSET))


        def _parse_revoked_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        revoked_at = _parse_revoked_at(d.pop("revoked_at", UNSET))


        def _parse_revoked_by(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        revoked_by = _parse_revoked_by(d.pop("revoked_by", UNSET))


        def _parse_rotated_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        rotated_at = _parse_rotated_at(d.pop("rotated_at", UNSET))


        def _parse_previous_secret_valid_until(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        previous_secret_valid_until = _parse_previous_secret_valid_until(d.pop("previous_secret_valid_until", UNSET))


        api_token_response = cls(
            id=id,
            name=name,
            prefix=prefix,
            scope=scope,
            created_by=created_by,
            tenant_id=tenant_id,
            permissions=permissions,
            legacy_full_access=legacy_full_access,
            created_at=created_at,
            expires_at=expires_at,
            last_used_at=last_used_at,
            revoked_at=revoked_at,
            revoked_by=revoked_by,
            rotated_at=rotated_at,
            previous_secret_valid_until=previous_secret_valid_until,
        )


        api_token_response.additional_properties = d
        return api_token_response

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
