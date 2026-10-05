from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="AppTokenRequest")



@_attrs_define
class AppTokenRequest:
    """ What `POST /auth/app/token` takes, as JSON or a form (RFC 6749 §4.1.3,
    §6).

        Attributes:
            grant_type (str): `authorization_code` or `refresh_token`.
            code (None | str | Unset):
            code_verifier (None | str | Unset):
            redirect_uri (None | str | Unset):
            client_id (None | str | Unset):
            refresh_token (None | str | Unset):
            client_name (None | str | Unset): With `authorization_code`: the app, as `/auth/app/sessions` takes it.
            install_id (None | str | Unset):
            platform (None | str | Unset):
            version (None | str | Unset):
     """

    grant_type: str
    code: None | str | Unset = UNSET
    code_verifier: None | str | Unset = UNSET
    redirect_uri: None | str | Unset = UNSET
    client_id: None | str | Unset = UNSET
    refresh_token: None | str | Unset = UNSET
    client_name: None | str | Unset = UNSET
    install_id: None | str | Unset = UNSET
    platform: None | str | Unset = UNSET
    version: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        grant_type = self.grant_type

        code: None | str | Unset
        if isinstance(self.code, Unset):
            code = UNSET
        else:
            code = self.code

        code_verifier: None | str | Unset
        if isinstance(self.code_verifier, Unset):
            code_verifier = UNSET
        else:
            code_verifier = self.code_verifier

        redirect_uri: None | str | Unset
        if isinstance(self.redirect_uri, Unset):
            redirect_uri = UNSET
        else:
            redirect_uri = self.redirect_uri

        client_id: None | str | Unset
        if isinstance(self.client_id, Unset):
            client_id = UNSET
        else:
            client_id = self.client_id

        refresh_token: None | str | Unset
        if isinstance(self.refresh_token, Unset):
            refresh_token = UNSET
        else:
            refresh_token = self.refresh_token

        client_name: None | str | Unset
        if isinstance(self.client_name, Unset):
            client_name = UNSET
        else:
            client_name = self.client_name

        install_id: None | str | Unset
        if isinstance(self.install_id, Unset):
            install_id = UNSET
        else:
            install_id = self.install_id

        platform: None | str | Unset
        if isinstance(self.platform, Unset):
            platform = UNSET
        else:
            platform = self.platform

        version: None | str | Unset
        if isinstance(self.version, Unset):
            version = UNSET
        else:
            version = self.version


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "grant_type": grant_type,
        })
        if code is not UNSET:
            field_dict["code"] = code
        if code_verifier is not UNSET:
            field_dict["code_verifier"] = code_verifier
        if redirect_uri is not UNSET:
            field_dict["redirect_uri"] = redirect_uri
        if client_id is not UNSET:
            field_dict["client_id"] = client_id
        if refresh_token is not UNSET:
            field_dict["refresh_token"] = refresh_token
        if client_name is not UNSET:
            field_dict["client_name"] = client_name
        if install_id is not UNSET:
            field_dict["install_id"] = install_id
        if platform is not UNSET:
            field_dict["platform"] = platform
        if version is not UNSET:
            field_dict["version"] = version

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        grant_type = d.pop("grant_type")

        def _parse_code(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        code = _parse_code(d.pop("code", UNSET))


        def _parse_code_verifier(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        code_verifier = _parse_code_verifier(d.pop("code_verifier", UNSET))


        def _parse_redirect_uri(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        redirect_uri = _parse_redirect_uri(d.pop("redirect_uri", UNSET))


        def _parse_client_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        client_id = _parse_client_id(d.pop("client_id", UNSET))


        def _parse_refresh_token(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        refresh_token = _parse_refresh_token(d.pop("refresh_token", UNSET))


        def _parse_client_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        client_name = _parse_client_name(d.pop("client_name", UNSET))


        def _parse_install_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        install_id = _parse_install_id(d.pop("install_id", UNSET))


        def _parse_platform(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        platform = _parse_platform(d.pop("platform", UNSET))


        def _parse_version(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        version = _parse_version(d.pop("version", UNSET))


        app_token_request = cls(
            grant_type=grant_type,
            code=code,
            code_verifier=code_verifier,
            redirect_uri=redirect_uri,
            client_id=client_id,
            refresh_token=refresh_token,
            client_name=client_name,
            install_id=install_id,
            platform=platform,
            version=version,
        )


        app_token_request.additional_properties = d
        return app_token_request

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
